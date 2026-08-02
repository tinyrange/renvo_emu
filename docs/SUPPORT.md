# Renvo Emulator support contract

This document describes implemented behavior, not the long-term intent in
`PLAN.html`. “Functional” means deterministic and useful for the named corpus;
it does not mean cycle accuracy or complete silicon compatibility.

## Portfolio

| Target | Runnable CPU mode | Direct-load memory | Chip-facing proof |
|---|---|---|---|
| CH32V003 | QingKe-flavoured RV32EC/Zicsr subset | 16 KiB flash, 2 KiB SRAM | RCC + native GPIO, USART1, TIM2, PFIC and table-mode interrupt proofs |
| CH32V006 | QingKe-flavoured RV32EC/Zicsr subset | 64 KiB flash, 8 KiB SRAM | Native WCH RCC/GPIO/USART1/TIM2/PFIC slice with independently sized map |
| RP2040 | Cortex-M0+ Armv6-M Thumb subset | 16 MiB XIP window, 264 KiB SRAM | SIO GPIO25 waveform; native UART0 transcript; PL011 UART1 register slice; native TIMER→NVIC; PIO0 `SET PINS` waveform |
| RP2350 | Cortex-M33 Thumb subset or Hazard3 RV32IMAC/B subset | 16 MiB XIP window, 520 KiB SRAM | SIO GPIO, UART0, PL011 UART1 register slice, TIMER interrupt, and PIO0 waveform proofs in both CPU modes |
| ESP32-S3 | Xtensa LX7 windowed compiler subset | DRAM, IRAM, 16 MiB IROM and DROM windows | Windowed ABI/exception/atomic/FPU qualification; GPIO matrix low bank waveform and native-address UART0 FIFO transcript |
| ESP32-C6 | RV32IMAC/Zicsr machine/user subset | ROM, HP/LP SRAM, 16 MiB IROM window | GPIO matrix pin 2 waveform, native UART0 transcript, user traps, and PMP CSR visibility |

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
nesting/HPE and PMP permission enforcement remain unsupported. ESP32-C6 supports
machine/user transitions, user ECALL, illegal privileged-CSR traps, interrupt
entry, MRET, and PMP CSR visibility. `qualification/riscv-cpu.json` contains
Docker and negative-profile proofs for XW, Zmmul, ESP privilege/trap behavior,
and the complete RP2350 compiler `-march` baseline.

The Arm interpreter covers the 16-bit Thumb compiler baseline, BL, Thumb-2 and
DSP forms observed in Cortex-M33 output, FPv5 single-precision compiler output,
stack operations, four-state run stops, WFI, functional exception
stacking/vectoring, and EXC_RETURN. The private-peripheral model includes a
deterministic SysTick and all eight NVIC enable/pending banks (240 external
lines). It does not cover the full Armv8-M instruction set, TrustZone, MPU,
NVIC priority/preemption, or complete system-control register surface.

The Xtensa interpreter covers 16-bit density and 24-bit compiler instruction
forms, core integer ALU and comparisons, multiply/divide, SAR-based shifts,
branches, jump-table dispatch, and zero-overhead loops. It implements logical
register windows for the Espressif toolchain's default windowed ABI, level-one
interrupt entry and `RFE`, `S32C1I`/`SCOMPARE1` atomics, and the
single-precision FPU operations emitted by the qualification workload.
`qualification/xtensa-cpu.json` records pinned-GCC proofs at `-O0`, `-O2`, and
`-Os`, including byte-identical repeat runs and IRAM/DRAM/IROM/DROM execution.
Precise window-overflow traps, complete interrupt priority/nesting, and the
full optional Xtensa ISA remain outside the functional baseline.

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
timer-interrupt cases, the full RP2350 Hazard3 compiler ISA gate, and the Rust
ABI matrix. It also runs the Arm SysTick/multi-bank NVIC and Cortex-M33
hard-float/DSP gates recorded in `qualification/arm-cpu.json`.
The same gate also runs the windowed Xtensa ABI, exception, atomic, FPU, memory
view, and deterministic-repeat matrix recorded in
`qualification/xtensa-cpu.json`.

The final distillation gate adds four bounded capabilities:

- `remu corpus reduce` minimizes source fragments, flags and numeric inputs,
  recompiling every predicate inside the pinned Docker toolchain. The checked
  three-family evidence is `qualification/reduction.json`.
- `remu script` evaluates Starlark assertions over caller-selected JSON only;
  scripting never owns a machine or kernel state. Its portfolio proof is
  `qualification/starlark.json`.
- `remu board` resolves loaded Starlark board definitions into immutable
  topology/actions, then hands that scenario to the Rust board runner. It does
  not expose live CPU, scheduler, or peripheral state to the Starlark VM. See
  `docs/BOARD_SIMULATION.md` and `scripts/qualify-board-models.sh`.
- `remu gdb` serves one GDB remote-protocol session with registers, memory,
  breakpoints, step and continue. Direct runs additionally accept `--coverage`
  and `--replay`. RISC-V, Arm and Xtensa evidence is in
  `qualification/debug-observability.json`.
- `scripts/generate-dashboard.sh` fails unless all six target manifests,
  register manifests and Phase 5 artifacts pass, then regenerates
  `qualification/dashboard.html` and `.json`.

## Unmodified vendor sample gate

`scripts/qualify-vendor-samples.sh` fetches three official files at immutable
Git commits and rejects any SHA-256 mismatch: WCH EVT `GPIO_Toggle`, Raspberry
Pi `blink_simple`, and ESP-IDF `hello_world`. The upstream C files are not
patched. Tracked startup and narrow SDK compatibility adapters provide the
direct-ELF boundary and native WCH GPIO/USART, RP SIO GPIO, or ESP UART MMIO.

The samples compile only inside the pinned Docker images and pass on
CH32V003, CH32V006, RP2040, RP2350 Arm, RP2350 Hazard3, ESP32-S3 and ESP32-C6.
`qualification/vendor-samples.json` records repositories, commits, file paths,
source hashes, build/run hashes and licence treatment. WCH source is fetched on
demand and not redistributed because its file notice restricts its use;
Pico's sample is BSD-3-Clause and the ESP-IDF sample is CC0-1.0.

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

Direct-run `--bus-log` output is streamed as an ordered JSON array, so its
memory use is bounded independently of the number of accesses. The schema and
pretty-printed ordering remain compatible with existing qualification
artifacts. Coverage uses the same event stream but retains only unique executed
addresses.

For ESP32-C6, direct ELF loading proves instruction and peripheral behavior but
does not exercise the second-stage bootloader's flash mappings. Supplying the
corresponding esptool application binary with `--esp-app-image` enables a
separate boot-layout gate. It checks the chip and entry metadata, descriptor
and text segment ordering, 64 KiB mapping congruence, and correspondence with
the executable ELF. The default application partition offset is `0x10000` and
can be changed with `--esp-app-offset`.

ESP32-C6 application RAM powers on with the deterministic nonzero byte pattern
`0xa5`. Direct ELF loading copies only the file-backed portion of writable load
segments, leaving each `p_memsz - p_filesz` tail poisoned. Firmware must
therefore perform its own `.bss` initialization, as it must on hardware. Other
targets retain their existing reset-memory policy.

The native-image boundary covers every target advertised by `remu targets`.
RP targets consume UF2 and preserve their Arm/RISC-V boot selection; Espressif
targets consume validated merged flash images (or the existing application-UF2
overlay path); PIC16 and EFM8 consume Intel HEX; the remaining byte-addressed
MCUs consume Intel HEX or addressless raw binaries rooted at their documented
primary flash base. Native loading uses vector/reset semantics where the
architecture defines them instead of inventing an ELF entry point.

Native/direct equivalence is continuously checked by
`scripts/qualify-native-images.sh`. All compiler inputs are built in immutable,
network-disabled Docker toolchains. The gate compares stop reason, exit code,
UART/USB output, trace digest, and byte-identical VCD for all 14 target modes.
PIC16 and EFM8 use their direct Intel HEX boundary because their toolchains do
not emit a runnable ELF; every other mode compares native boot with ELF.

The human-readable portfolio view is
[`qualification/dashboard.html`](../qualification/dashboard.html), with the
same checked data in `qualification/dashboard.json`. “Baseline proven” there
means a deterministic functional compiler/firmware model, never complete
silicon compatibility or cycle accuracy.
