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
| 0 — Kernel contracts and manifests | Partial | Workspace crates, checked `SimTime`, deterministic event queue, bus/device/CPU/trace contracts, four ADRs, and six source-linked manifests | Fake-multicore insertion-order stress and supported-host repeat evidence |
| 1 — RISC-V family | Partial | Docker GCC/Clang corpus, CoreMark, direct ELF execution, CH32V003/006 PFIC table entry, ESP32-C6 and RP2350 Hazard3 profiles | Remaining WCH XW behavior, fuller ESP privilege/PMP proof, and explicit breakpoint/watchpoint gate |
| 2 — Arm M-profile | Partial | RP2040 and RP2350 Arm run Docker corpus, CoreMark and official firmware; exception and multicore paths have focused tests | Required M33 DSP/FPU closure, fuller NVIC proof, and plan-specific C/Rust ABI gate |
| 3 — Xtensa LX7 | Partial | ESP32-S3 runs Docker GCC corpus, CoreMark and official firmware; register windows and task switching have focused tests | Remaining exception, atomic and FPU behavior plus the plan's optimization/ABI gate |
| 4 — Peripheral and VCD baseline | Partial | Four-state signals, scheduled input, VCD, native WCH GPIO/USART/TIM2/PFIC, native RP GPIO/timer/UART paths on all three CPU profiles, native ESP GPIO/timer/UART paths, and official-firmware peripheral use | RP PIO proof and per-chip register coverage manifests |
| 5 — Distillation and selective depth | Partial | Immutable Docker builds, GCC/Clang matrices, 1,000 distinct C cases, comparison API, reduction primitive, CoreMark and stable JSON artifacts | Rust compiler lane, selected unmodified vendor samples, seeded end-to-end reduction on all three CPU families, Starlark, GDB and coverage dashboard |

## Six-chip baseline definition

| Requirement from `PLAN.html` | Status | Evidence or gap |
|---|---|---|
| Compiler-produced ELF on every CPU profile | Proven | `scripts/edge-corpus.sh` runs seven target/CPU combinations |
| Memory maps, reset, traps and interrupt entry | Partial | All maps and functional entry paths exist; CPU-specific limitations remain in `renvo targets --json` |
| Per-chip flash/RAM/MMIO, timer, GPIO, UART and IRQ routing | Proven | Docker smoke covers native-address GPIO/UART and explicit WCH/RP timer interrupt paths; official MicroPython callbacks cover the ESP timer-group routes |
| Scheduled pin input and resolved digital nets | Proven | Signal/device unit tests and MicroPython external-input qualification |
| Stable hierarchical VCD | Proven | Trace unit tests, Docker smoke VCDs and official-firmware qualification |
| Exit, fault, breakpoint, signal edge, virtual-time and instruction stops | Partial | Stop reasons exist; public breakpoint/watchpoint and signal-edge gates are incomplete |
| Stable machine-readable result and event digest | Proven | CLI JSON artifacts and deterministic trace digests |
| Selected unmodified WCH EVT, Pico SDK and ESP-IDF samples | Missing | Official MicroPython is valuable additional evidence but does not replace this named gate |
| GCC, Clang and Rust across optimization levels | Partial | GCC and Clang pass; the target Rust lane is missing |
| Compare selected output and flag divergence | Proven | `renvo corpus compare` and comparison unit tests |
| Reduce seeded divergence on RISC-V, Arm and Xtensa | Missing | Deterministic reducer primitive exists, but no three-family end-to-end proof |
| Publish coverage, fidelity, unsupported behavior, provenance and licences | Partial | Source-linked target manifests and build provenance exist; register coverage/licence dashboard is missing |
| Identical results across supported hosts | Partial | Repeated local digests exist; supported-host matrix is not published |
| Separate CPU/device/trace/script/CLI boundaries | Partial | Rust crate boundaries are clean; the Starlark scripting boundary is not implemented |

## Most recent closure

RP2040, RP2350 Arm, and RP2350 Hazard3 now run Docker-built firmware that
programs native TIMER alarm, interrupt-enable, and status registers. Each
profile enters `WFI`, takes its NVIC or Hazard3-routed interrupt, clears the
alarm, and exits with code 0 after exactly one recorded event.

## Previous closure

Docker-built firmware now writes the chip UART0/FIFO addresses on RP2040,
RP2350 Arm, RP2350 Hazard3, ESP32-S3, and ESP32-C6. Together with the existing
WCH USART proof, all seven target/CPU combinations produce an exact checked
transcript through native-address MMIO. RP firmware also polls the PL011 flag
register rather than relying on the compiler UART facade.

## Earlier closure

The WCH slice now includes deterministic TIM2 update timing, PFIC enable and
pending registers, QingKe CSR `0x804`, and PFIC table-mode interrupt entry via
`mtvec` mode 3. Docker-built RV32EC firmware configures the native TIM2 and
PFIC addresses, enters `WFI`, services interrupt 38 through its vector-table
entry, clears the vendor status flag, and exits successfully on both CH32V003
and CH32V006. `scripts/docker-smoke.sh` checks the exit code and single event.

The preceding USART1 closure remains covered by the same gate: native
`STATR`, `DATAR`, `BRR`, `CTLR1/2/3`, and `GPR` accesses produce the exact
`RENVO-WCH\n` transcript on both WCH targets.
