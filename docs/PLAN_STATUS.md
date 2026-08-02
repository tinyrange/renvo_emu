# Restored plan status

This ledger tracks the restored `PLAN.html` and nothing broader. Features that
the plan marks **deliberately later**, **deferred from baseline**, or
**corpus-driven next** are not completion blockers unless an exit gate names
them explicitly.

Status meanings:

- **Proven**: an executable check covers the stated requirement.
- **Partial**: useful implementation exists, but the plan's full gate is not
  yet demonstrated.
- **Missing**: no adequate end-to-end evidence exists yet.

## Phase gates

| Phase | Status | Current evidence | Required closure |
|---|---|---|---|
| 0 — Kernel contracts and manifests | Proven | Workspace contracts and ADRs; six source-linked manifests; fake dual-core/timer canonical digest across 64 repeat/insertion-stress variants on pinned Linux/amd64 and Linux/arm64 environments | None |
| 1 — RISC-V family | Proven | Docker GCC/Clang corpus, exact-RV32E and RV32IMAC Rust ABI matrix, CoreMark, QingKe XW/Zmmul, CH32V003/006 PFIC table entry, ESP32-C6 machine/user traps and PMP CSR visibility, typed stops, and the complete RP2350 Hazard3 compiler `-march` harness | None |
| 2 — Arm M-profile | Proven | RP2040 and RP2350 pass Docker C/Rust ABI and CoreMark suites; both take SysTick and bank-1 NVIC exceptions with architectural stacking/return; RP2350 runs compiler-emitted hard-float FPv5 and DSP code; Cortex-M33/Hazard3 share the same Rust computation matrix | None |
| 3 — Xtensa LX7 | Proven | Pinned Espressif GCC emits and Renvo Emulator executes windowed ABI calls, register windows, S32C1I atomics, single-precision FPU code, level-one exception entry/RFE, and all four ESP32-S3 ELF memory views at `-O0`, `-O2` and `-Os`; every run repeats byte-identically | None |
| 4 — Peripheral and VCD baseline | Proven | Four-state signals, scheduled input, stable VCD, native WCH GPIO/USART/TIM2/PFIC, native RP GPIO/timer/UART/PIO paths on all three CPU profiles, native ESP GPIO/timer/UART paths, official-firmware peripheral use, and six generated register-coverage/deviation manifests | None |
| 5 — Distillation and selective depth | Proven | Immutable Docker builds; GCC/Clang/Rust matrices; 1,000 distinct C cases; comparison and three-axis reduction; hash-pinned unmodified WCH EVT, Pico SDK and ESP-IDF samples; bounded Starlark assertions; GDB RSP; coverage/replay; and the six-target fidelity dashboard | None |

## Six-chip baseline definition

| Requirement from `PLAN.html` | Status | Evidence or gap |
|---|---|---|
| Compiler-produced ELF on every CPU profile | Proven | `scripts/edge-corpus.sh` runs seven target/CPU combinations |
| Memory maps, reset, traps and interrupt entry | Proven | The three CPU-family qualification artifacts prove the required direct maps and functional entry paths; fidelity beyond the baseline remains explicit in `remu targets --json` |
| Per-chip flash/RAM/MMIO, timer, GPIO, UART and IRQ routing | Proven | Docker smoke covers native-address GPIO/UART and explicit WCH/RP timer interrupt paths; official MicroPython callbacks cover the ESP timer-group routes |
| Scheduled pin input and resolved digital nets | Proven | Signal/device unit tests and MicroPython external-input qualification |
| Stable hierarchical VCD | Proven | Trace unit tests, Docker smoke VCDs and official-firmware qualification |
| Exit, fault, breakpoint, signal edge, virtual-time and instruction stops | Proven | CLI controls and `qualification/stop-conditions.json` prove all non-exit stops independently on RISC-V, Arm and Xtensa; normal portfolio smoke proves exit |
| Stable machine-readable result and event digest | Proven | CLI JSON artifacts and deterministic trace digests |
| Selected unmodified WCH EVT, Pico SDK and ESP-IDF samples | Proven | `qualification/vendor-samples.json` binds exact upstream commits and source hashes; the byte-exact sources compile in pinned Docker toolchains and run through native WCH GPIO, RP SIO GPIO and ESP UART MMIO on all seven CPU profiles |
| GCC, Clang and Rust across optimization levels | Proven | GCC/Clang C matrices pass; `qualification/rust-abi.json` proves exact RV32E, RV32IMAC, Armv6-M and Armv8-M Rust targets at `-O0`, `-O2` and `-Os` across all six applicable CPU profiles; `qualification/xtensa-cpu.json` proves Xtensa GCC at the same levels. |
| Compare selected output and flag divergence | Proven | `remu corpus compare` and comparison unit tests |
| Reduce seeded divergence on RISC-V, Arm and Xtensa | Proven | `qualification/reduction.json` records every Docker build/run evaluation and the one-item source/flag/input reproducer on CH32V003, RP2040 and ESP32-S3; final repeats are identical |
| Publish coverage, fidelity, unsupported behavior, provenance and licences | Proven | `qualification/dashboard.html` and `.json` combine six source-linked target manifests, passing corpus, generated register coverage, known deviations and sample licence/provenance without claiming cycle or full-silicon fidelity |
| Identical results across supported hosts | Proven | `scripts/qualify-host-determinism.sh` publishes the same canonical fake-multicore/timer digest on the supported Linux/amd64 and Linux/arm64 hosts |
| Separate CPU/device/trace/script/CLI boundaries | Proven | CPU, machine, device, trace, corpus, GDB and Starlark crates remain separate; `remu script` evaluates explicit JSON, while the later board DSL builds immutable scenarios without owning kernel state |

## Phase 5 closure

## ATSAMD21 DMAC expansion slice

The ATSAMD21 model now maps the documented DMAC block at `0x41004800` and
uses named register and descriptor-field enums rather than numeric register
identifiers. The register masks and access behavior follow the official SAM
D21 definitions: the reserved `0x0e` slot is not fabricated as QOSCTRL,
CTRL/PRICTRL0/CHCTRLB masks are bounded, direct writes can disable DMA and
channels, and software-trigger/INTPEND/CHINTEN/CHINTFLAG payloads retain their
documented write semantics. All twelve channel selectors, enable/reset and
trigger/event configuration fields, software-trigger bitmap, base/write-back
addresses, pending/busy/active summaries, interrupt enable/flag/status paths,
and write-one-to-clear behavior are modeled. A valid software-triggered
descriptor can transfer a bounded single block through the shared bus for byte,
halfword, or word memory-to-memory traffic; source/destination incrementing and
the descriptor write-back layout are deterministic. Invalid descriptors and
bus faults latch fetch/transfer errors and interrupt flags.

The Docker ATSAMD21 smoke now programs a native-address descriptor, verifies
the copied bytes and completion state, and requires DMAC bus evidence. The
official SAM D21 datasheet remains the register and descriptor reference:
<https://ww1.microchip.com/downloads/en/DeviceDoc/SAM_D21_DA1_Family%20Data%20Sheet_DS40001882E.pdf>.
Linked descriptors, peripheral and Event System trigger routing, CRC
calculation, exact arbitration/timing, and other analog/USB blocks remain
outside this functional tranche.

Phase 5 now meets its complete exit gate. `remu corpus reduce` detects the
seeded discrepancy and minimizes source fragments, compiler flags, and inputs
on RISC-V, Arm, and Xtensa; all 45 predicate evaluations retain Docker build
and run provenance, and each minimized case repeats identically. The bounded
Starlark layer asserts over explicit JSON datasets without coupling scripting
values to the simulation kernel.

The direct runner publishes deterministic symbolicated instruction coverage
and checks whole-result replay. The optional `remu-gdb` crate and CLI serve a
standards-level GDB remote session; the qualification reads registers and
memory, inserts/removes a breakpoint, and single-steps each CPU family. The
upstream sample gate downloads source from pinned WCH EVT, Pico examples, and
ESP-IDF commits, verifies SHA-256, and compiles each file byte-exact in the
pinned containers. Tracked SDK adapters route those programs to native GPIO or
UART MMIO on all seven CPU profiles.

`qualification/dashboard.html` and `qualification/dashboard.json` are the
six-target exit artifact. They report functional support tier, passing corpus,
observed register coverage, sources, licences, and known gaps. The complete
Docker portfolio gate regenerates this evidence in 29 seconds on each of two
final repeated runs on the current host; the complete `qualification/` trees
were byte-identical.

## Previous Phase 3 closure

Phase 3 now meets its complete exit gate. The pinned Espressif GCC default
windowed ABI generates nested `call8`, `ENTRY`, and `RETW` sequences which run
successfully at `-O0`, `-O2`, and `-Os`. The same freestanding ELF proof uses
compiler-generated `S32C1I` atomics and single-precision FPU operations, takes
a guest-raised level-one exception, returns through `RFE`, and executes or
reads sections in IRAM, DRAM, IROM, and DROM. Each variant runs twice with an
identical result, while a focused machine test proves CPU1 starts reset and
parked. Build, disassembly, ELF, result, and unit-test hashes are bound in
`qualification/xtensa-cpu.json`.

## Previous Phase 2 closure

Phase 2 meets its complete exit gate. The private-peripheral model advances
a deterministic 24-bit SysTick and exposes all eight NVIC enable/pending banks
for 240 external lines. Docker-built RP2040 and RP2350 ELFs take SysTick and
bank-1 software-pended interrupts, stack architectural state, return through
EXC_RETURN, and exit successfully. A hard-float Cortex-M33 ELF compiled with
`-mfpu=fpv5-sp-d16 -mfloat-abi=hard` executes compiler-emitted fused
single-precision arithmetic/comparison and DSP multiply-accumulate. The checked
`qualification/arm-cpu.json` also binds the existing O0/O2/Os Rust computation
that passes in both RP2350 Cortex-M33 and Hazard3 modes.

## Previous Phase 1 closure

Phase 1 now meets its complete exit gate. ESP32-C6 implements named
machine/user privilege state, ECALL and illegal privileged-CSR traps, MRET,
user-mode interrupt entry, and PMP CSR visibility; a Docker-built ELF proves
those transitions. The RP2350 Hazard3 harness makes GCC accept the documented
`rv32imac_zicsr_zifencei_zba_zbb_zbs_zbkb_zcb_zcmp` string, executes the
compiler-generated cases, and runs explicit Zbkb, Zcb, and Zcmp operations.
All build inputs, compiler outputs, ELFs, run results, and focused negative
tests are hashed in `qualification/riscv-cpu.json`.

## Previous RISC-V closure

The QingKe profiles now implement all eight XW compressed byte/halfword memory
operations named by the WCH V2 manual. CH32V006 separately enables the V2C
multiply-only Zmmul subset while CH32V003 rejects it and V2C continues to reject
divide. Docker-built raw-opcode firmware passes on both WCH chips, and focused
negative tests prove profile gating. Source, ELF, build, result, and unit-test
hashes are collected in `qualification/riscv-cpu.json`.

## Previous closure

The pinned Rust 1.97.1 image now provides upstream bare-metal libraries for
RV32IMAC, Armv6-M, and Armv8-M plus an image-prebuilt `core` for the exact
`riscv32e-unknown-none-elf` QingKe register ABI. A freestanding Rust program exercises
slice iteration, structure passing, a five-argument C ABI boundary, calls,
rotates, wrapping arithmetic, and static data. Docker compiles it at `-O0`,
`-O2`, and `-Os`; CH32V003, CH32V006, ESP32-C6, RP2040, RP2350 Arm, and RP2350
Hazard3 each run every variant twice with byte-identical results. The 18 proof
rows, compiler/container provenance, and hashes are in
`qualification/rust-abi.json`.

## Earlier stop-condition closure

The public direct-run CLI now accepts typed breakpoints, data watchpoints, and
named change/rising/falling signal stops. Arm `BKPT` and Xtensa `BREAK` report
the architectural breakpoint reason rather than masquerading as a halt, while
RISC-V retains `EBREAK`. Watchpoints cover overlapping completed reads/writes
without triggering on instruction fetch. Eighteen Docker-built proofs cover
fault, breakpoint, watchpoint, signal edge, virtual-time deadline, and
instruction-budget stops independently on RISC-V, Arm, and Xtensa; ordinary
portfolio runs cover explicit exit. Their stable results and hashes are in
`qualification/stop-conditions.json`.

## Earlier Phase 0 closure

Phase 0 now has executable host-independent determinism evidence. A fake
dual-core machine advances both CPUs in stable round-robin order alongside a
timer, while 64 variants perturb event insertion IDs with same-time cancelled
events. Its fixed canonical digest is checked over 64 repeats in pinned
Linux/amd64 and Linux/arm64 Rust containers. Both build and execute the test
for their selected architecture and publish the same digest in
`qualification/host-determinism.json`; arm64 may be native or supplied through
binfmt/QEMU on a development host.

## Earlier register-coverage closure

The Docker portfolio gate now records every completed bus access and generates
one checked coverage manifest per chip. Each manifest contains only registers
observed in passing firmware, hashes its complete run and bus-log evidence,
and publishes the model's known deviations. The generator requires all six
target identities and fails if any required GPIO, UART, timer/interrupt, PIO,
or WCH-specific region disappears from tested coverage. Repeated generation
is byte-for-byte deterministic.

## Earlier PIO closure

RP2040, RP2350 Arm, and RP2350 Hazard3 now run Docker-built firmware against
native PIO0 registers. The functional PIO model covers instruction memory,
state-machine configuration, direct execution, unconditional `JMP`, and `SET`
to pins, pin directions, X, and Y. The smoke program executes a two-instruction
`SET PINS` loop on GPIO25, and all three profiles produce a checked hierarchical
PIO VCD scope and exit with code 0. One PIO instruction advances per abstract
tick; FIFO, IRQ, `WAIT`, shift, side-set, delay fields, clock-divider timing,
and PIO v1 extensions are explicit known deviations.

## Earlier RP timer closure

RP2040, RP2350 Arm, and RP2350 Hazard3 now run Docker-built firmware that
programs native TIMER alarm, interrupt-enable, and status registers. Each
profile enters `WFI`, takes its NVIC or Hazard3-routed interrupt, clears the
alarm, and exits with code 0 after exactly one recorded event.

## Earlier UART closure

Docker-built firmware now writes the chip UART0/FIFO addresses on RP2040,
RP2350 Arm, RP2350 Hazard3, ESP32-S3, and ESP32-C6. Together with the existing
WCH USART proof, all seven target/CPU combinations produce an exact checked
transcript through native-address MMIO. RP firmware also polls the PL011 flag
register rather than relying on the compiler UART facade.

## Earlier WCH closure

The WCH slice now includes deterministic TIM2 update timing, PFIC enable and
pending registers, QingKe CSR `0x804`, and PFIC table-mode interrupt entry via
`mtvec` mode 3. Docker-built RV32EC firmware configures the native TIM2 and
PFIC addresses, enters `WFI`, services interrupt 38 through its vector-table
entry, clears the vendor status flag, and exits successfully on both CH32V003
and CH32V006. `scripts/docker-smoke.sh` checks the exit code and single event.

The preceding USART1 closure remains covered by the same gate: native
`STATR`, `DATAR`, `BRR`, `CTLR1/2/3`, and `GPR` accesses produce the exact
`REMU-WCH\n` transcript on both WCH targets.
