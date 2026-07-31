# Renvo support contract

This document describes implemented behavior, not the long-term intent in
`PLAN.html`. “Functional” means deterministic and useful for the named corpus;
it does not mean cycle accuracy or complete silicon compatibility.

## Portfolio

| Target | Runnable CPU mode | Direct-load memory | Chip-facing proof |
|---|---|---|---|
| CH32V003 | QingKe-flavoured RV32EC/Zicsr subset | 16 KiB flash, 2 KiB SRAM | RCC + native GPIO, USART1, TIM2, PFIC and table-mode interrupt proofs |
| CH32V006 | QingKe-flavoured RV32EC/Zicsr subset | 64 KiB flash, 8 KiB SRAM | Native WCH RCC/GPIO/USART1/TIM2/PFIC slice with independently sized map |
| RP2040 | Cortex-M0+ Armv6-M Thumb subset | 16 MiB XIP window, 264 KiB SRAM | SIO GPIO25 waveform; native UART0 transcript; native TIMER→NVIC; PIO0 `SET PINS` waveform |
| RP2350 | Cortex-M33 Thumb subset or Hazard3 RV32IMAC/B subset | 16 MiB XIP window, 520 KiB SRAM | SIO GPIO, UART0, TIMER interrupt, and PIO0 waveform proofs in both CPU modes |
| ESP32-S3 | Xtensa LX7 compiler-emitted subset | DRAM, IRAM, 16 MiB IROM window | GPIO matrix low bank waveform and native-address UART0 FIFO transcript |
| ESP32-C6 | RV32IMAC/Zicsr subset | ROM, HP/LP SRAM, 16 MiB IROM window | GPIO matrix pin 2 waveform and native-address UART0 FIFO transcript |

All targets also expose a stable compiler-test block:

- GPIO at `0xffff0000`
- UART at `0xffff0100`
- functional timer at `0xffff0200`
- exit word at `0xfffffff0`

This block is explicitly a compiler facade, separate from chip register
compatibility. It lets architecture tests share stopping and observation
conventions without pretending that vendor peripherals are interchangeable.

## Official MicroPython milestone

Official, unmodified MicroPython v1.28.0 firmware reaches its native USB raw
REPL on NanoC6, AtomS3 Lite, Pico, and both Pico 2 CPU modes. The current
acceptance matrix proves deterministic portable runtime behavior, soft reset,
threads, persistent storage, external GPIO stimulus, and periodic/one-shot
timer callbacks across all five execution profiles. RP2040, both RP2350 timer
blocks, ESP32-C6 timer groups, and both ESP32-S3 timer groups have distinct
functional register layouts and interrupt delivery.

The host transport counts raw-REPL execution terminators and returned prompts,
so a bounded run ends only after every queued chunk has executed. Instruction
limits remain hard failure bounds rather than the normal completion mechanism.
See `scripts/qualify-micropython.sh` and
`qualification/acceptance-report.html`.

This milestone does not yet cover the complete upstream MicroPython suite,
PWM/ADC/serial buses, watchdog resets, or virtual ESP radio connectivity.

## Implemented CPU surface

The RISC-V interpreter covers RV32I/E integer execution, common compressed
instructions, M and A where selected by the profile, Zicsr, WFI/MRET, basic
machine interrupt entry, QingKe PFIC table-mode entry, all eight QingKe XW
compressed byte/halfword memory operations, the V2C multiply-only Zmmul
subset, and the compiler-facing Hazard3 bit-manipulation subset. Exact PFIC
nesting/HPE, PMP permission enforcement, and complete privilege behavior remain
unsupported. `qualification/riscv-cpu.json` contains Docker and negative-profile
proofs for XW and Zmmul.

The Arm interpreter covers the 16-bit Thumb compiler baseline, BL, selected
Thumb-2 immediate forms observed in Cortex-M33 output, stack operations,
four-state run stops, WFI, functional exception stacking/vectoring, and
EXC_RETURN. It does not yet cover the full Armv8-M, DSP, FPU, TrustZone, MPU, or
complete NVIC register surface.

The Xtensa interpreter covers the instructions emitted by the portable C
baseline in call0 mode, including L32R, MEMW, density forms, core integer ALU
and comparisons, multiply/divide, SAR-based shifts, conditional branches,
direct calls/returns, jump-table dispatch, and zero-overhead loops.
Register-window calls, exceptions, atomics, and the FPU remain incomplete.

## Timing and tracing

One completed instruction or architectural action advances one abstract tick.
Timers, PIO and external pin stimuli use that deterministic timeline. The
model is not tied to target clock frequency. Baseline PIO executes one
instruction per abstract tick and intentionally ignores divider and delay
timing. VCD uses one nanosecond per abstract tick as a display convention, not
a hardware timing claim.

Signals use `0`, `1`, high impedance, and unknown/contention states. Changes
are streamed, and declaration/change digests are stable for equivalent runs.
The CLI accepts scheduled input in `PIN=VALUE@TICK` form.

Direct runs accept repeatable `--breakpoint ADDRESS` and `--watchpoint ADDRESS`
controls plus `--stop-signal PATH=change|rising|falling`. Addresses may be
decimal or `0x`-prefixed hexadecimal. A breakpoint stops before executing the
named address. A watchpoint stops after a completed CPU data read/write that
overlaps the named byte, and records the access address and kind in JSON.
Signal stops use stable hierarchical paths and preserve the triggering change
in VCD/digest output. `scripts/qualify-stop-conditions.sh` checks every stop
class on RISC-V, Arm, and Xtensa.

The supported host matrix is Linux/amd64 and Linux/arm64. A pinned Rust
container runs the fake dual-core/timer scheduler test on both architectures;
each architecture checks 64 repeat and insertion-stress variants against the
same fixed digest. `scripts/qualify-host-determinism.sh` regenerates the
machine-readable evidence in `qualification/host-determinism.json`. A native
arm64 host or configured aarch64 binfmt handler is required for the arm64 lane.

## Compiler containment

Firmware compilation is never invoked directly on the host. The corpus runner
requires an immutable Docker digest or image ID and applies:

- `--pull=never` and `--network=none`
- a read-only container root
- dropped Linux capabilities and `no-new-privileges`
- read-only source and dedicated writable output mounts
- explicit CPU, memory, PID, and wall-time limits
- stable environment ordering and complete input/output SHA-256 provenance

The initial cross-GCC image contains Ubuntu-packaged RISC-V and Arm
bare-metal GCC. The Xtensa image verifies the SHA-256 of Espressif’s official
toolchain archive before extracting it. The Rust image pins Rust 1.97.1 and
prebuilds `core` for the otherwise undistributed exact RV32E register-ABI target while
installing the upstream RV32IMAC, Armv6-M, and Armv8-M libraries. Every actual
Rust case build remains network-isolated and read-only.

`scripts/qualify-rust-abi.sh` compiles one freestanding ABI/behavior program at
`-O0`, `-O2`, and `-Os`. Its 18 deterministic proof runs cover CH32V003,
CH32V006, ESP32-C6, RP2040, RP2350 Arm, and RP2350 Hazard3. The program exercises
calls, slice iteration, structure and five-argument ABI passing, rotates,
wrapping arithmetic, and static data. `qualification/rust-abi.json` records the
ELF, build, result, source, compiler-image, and repeat-run hashes.

`scripts/docker-smoke.sh` builds and runs all six targets, both RP2350 CPU
modes, real-register GPIO cases, native-address UART cases, native WCH/RP
timer-interrupt cases, and the Rust ABI matrix.

`corpus/edge_cases` adds 1,000 UB-free C behavioral cases with independently
generated expected values. `scripts/edge-corpus.sh` compiles them in five
Docker-only ELF layouts and runs seven CPU/target combinations. The checked
baseline passes all cases at `-O0`, `-O2`, and `-Os`.

## CoreMark qualification

`scripts/qualify-coremark.sh` compiles pinned upstream EEMBC CoreMark sources
inside the immutable Arm/RISC-V and Xtensa Docker toolchains. CH32V006,
RP2040, both RP2350 CPU modes, ESP32-S3, and ESP32-C6 pass both the standard
2,000-byte performance and validation seed sets. Results include exact CRCs,
abstract action counts, host elapsed time, ELF and run hashes, compiler
arguments, source commit, container IDs, and host identity.

CH32V003 cannot safely run the standard dataset in its 2 KiB SRAM: static data
uses 2,032 bytes and leaves 16 bytes for a stack. Its standard link is rejected
by an executable 512-byte stack-reserve assertion. It passes the distinct
upstream 1,200-byte profile-generation workload for behavioral coverage.
See [COREMARK.md](COREMARK.md) for scores and methodology.

## Research sources

Machine facts and register choices are based on the vendor sources linked from
the target manifests and `PLAN.html`, principally:

- WCH CH32V003/CH32V006 datasheets, CH32V00x reference material, QingKe V2
  processor manual, and the official
  [OpenWCH CH32V003 EVT sources](https://github.com/openwch/ch32v003)
- Raspberry Pi RP2040 and RP2350 datasheets and the official
  [Pico SDK PIO register definitions](https://github.com/raspberrypi/pico-sdk/blob/master/src/rp2040/hardware_regs/include/hardware/regs/pio.h)
- Espressif ESP32-S3 and ESP32-C6 datasheets and technical reference manuals
- Espressif’s official tool package index and crosstool-NG releases

Register behavior not covered by a passing firmware proof remains either
unmapped or explicitly approximate.

The generated per-chip register evidence lives in
`qualification/register-coverage/`. `scripts/docker-smoke.sh` records complete
bus logs, verifies the portfolio, and regenerates all six manifests. The
manifests list observed register addresses and access kinds, proof hashes, and
known functional deviations; an unlisted address is not implicitly claimed as
either supported or unsupported.
