# CoreMark qualification

Renvo Emulator runs the upstream [EEMBC CoreMark](https://github.com/eembc/coremark)
workload on all six initial MCUs. RP2350 is tested in both Cortex-M33 and
Hazard3 modes, giving seven execution profiles.

## Results

This snapshot was generated on 31 July 2026 with a release build of Renvo Emulator on
an Intel Core i7-1165G7 host. Each standard performance result used 250
iterations and the required 2,000-byte dataset. The score is the number of
emulated CoreMark iterations completed per host wall-clock second.

| MCU | Emulated CPU | Standard result | Host iterations/s | Iterations/M Renvo Emulator actions |
|---|---|---:|---:|---:|
| CH32V003 | QingKe V2A / RV32EC | Does not fit safely | — | — |
| CH32V006 | QingKe V2C / RV32EC | Passed | 8.879 | 1.279283 |
| RP2040 | Cortex-M0+ | Passed | 17.192 | 2.570117 |
| RP2350 | Cortex-M33 | Passed | 23.313 | 3.394884 |
| RP2350 | Hazard3 / RV32IMAC | Passed | 15.982 | 3.191454 |
| ESP32-S3 | Xtensa LX7 | Passed | 17.735 | 3.495416 |
| ESP32-C6 | RV32IMAC | Passed | 9.975 | 3.191454 |

These host-calibrated scores measure Renvo Emulator interpreter throughput on the named
host. They are useful for emulator performance regression testing, but they
are **not MCU silicon scores** and must not be compared with hardware
CoreMark/s or submitted as EEMBC results. Renvo Emulator is functionally timed: one
completed instruction or architectural action advances one abstract tick.

The action-normalized column is deterministic for a fixed Renvo Emulator build and
firmware artifact. The host column additionally includes each machine's bus
and device-model overhead. That is why RP2350 Hazard3 and ESP32-C6 have the
same compiled instruction stream and action count but different host scores.

## Correctness evidence

Every standard profile passed both upstream seed configurations:

| Run | Dataset | Seeds | List CRC | Matrix CRC | State CRC |
|---|---:|---|---|---|---|
| Performance | 2,000 bytes | `0, 0, 0x66` | `0xe714` | `0x1fd7` | `0x8e3a` |
| Validation | 2,000 bytes | `0x3415, 0x3415, 0x66` | `0xe3c1` | `0x0747` | `0x8d84` |

The benchmark printed `Correct operation validated` and Renvo Emulator halted with
exit code zero in all twelve standard executions. Both performance and
validation runs lasted more than ten host seconds in this snapshot.

The benchmark files are byte-identical to Git blobs at EEMBC commit
`1f483d5b8316753a742cbf5590caf5bd0a4e4777`. Renvo Emulator supplies only the permitted
port, startup, timer/UART/exit facade, and linker definitions. GCC compilation
runs in pinned, network-isolated Docker images:

- Arm and RISC-V:
  `sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736`
- Xtensa:
  `sha256:e0c54aeaae63f842234ec88f7b5a61b69bfa4d9005ba7490df47328e0dc9892f`

CoreMark exposed one missing ESP32-S3 instruction during qualification.
Renvo Emulator now implements Xtensa `MULA.AA.LL` signed low-halfword multiplication
with 40-bit accumulation, covered by a CPU unit test.

## CH32V003

CoreMark's standard 2,000-byte static dataset does not safely fit in the
CH32V003's 2 KiB SRAM. The linked image consumes 2,032 bytes of static RAM,
leaving 16 bytes for the stack. The qualification linker requires a modest
512-byte stack reserve and deliberately rejects that standard image.

For behavioral coverage, CH32V003 instead passes CoreMark's upstream
1,200-byte profile-generation workload with the expected `0x6a79`,
`0x5608`, and `0xe5a4` algorithm CRCs. Its observed host throughput was
32.173 iterations/s and 4.614803 iterations per million Renvo Emulator actions. This
is a reduced, differently seeded workload and is not a standard or
cross-profile-comparable CoreMark result.

## Reproduce

With the two pinned compiler images and CoreMark checkout cached locally:

```sh
COREMARK_OFFLINE=1 scripts/qualify-coremark.sh
```

The script:

1. verifies every benchmark source against its pinned upstream Git blob;
2. compiles every ELF inside the appropriate pinned Docker toolchain;
3. runs performance and validation seeds on every standard-capable profile;
4. proves the CH32V003 standard image violates the stack reserve;
5. measures host elapsed time, peak RSS, and result-artifact size with the
   `remu.benchmark-command.v1` runner and validates all reported CRCs; and
6. writes complete build, ELF, transcript, run, benchmark, hash, container, and host
   provenance under `.remu/qualification/coremark/`.

The latest machine-readable result is
`.remu/qualification/coremark/results.json`. Set `COREMARK_ITERATIONS` to a
larger value if a faster host completes a standard run in under ten seconds.

## Performance regression policy

`qualification/benchmarks/budgets.json` defines the comparison policy. A
baseline comparison allows 25% wall-time noise, 50% peak-RSS noise, and 25%
result-artifact growth. The budget checker never replaces the deterministic
CoreMark CRC or abstract-action score: those remain correctness evidence, while
host throughput and resource measurements are explicitly host-dependent.

The scheduled benchmark workflow can additionally set
`COREMARK_OBSERVABILITY_MODES=1`. It runs the same pinned RP2040 image with no
trace, VCD, instruction coverage, and streamed bus-log modes at a fixed action
limit. The bus-log record includes both its output size and peak RSS so a long
streaming run can demonstrate bounded emulator memory independently of the
number of completed instructions. These instrumentation profiles are reported
under `observability_modes`, not mixed into the CoreMark score.
