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
| ESP32-S3 | Xtensa LX7 windowed compiler subset | DRAM, IRAM, 16 MiB IROM and DROM windows | Windowed ABI/exception/atomic/FPU qualification; GPIO/UART proof plus functional I2C, SPI, I2S, and bidirectional RMT transactions; complete M5StickS3 non-radio board workflow |
| ESP32-C6 | RV32IMAC/Zicsr HP and LP cores | ROM, HP/LP SRAM, 16 MiB IROM window | Complete non-radio MMIO inventory, functional serial/timing/motor/audio/DMA/SDIO/analog/security slices, PMU/cache control, machine/user PLIC and CLINT, staged watchdog resets, user traps, and PMP enforcement |

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
complete PWM/serial-bus behavior or watchdog resets. ESP radio qualification uses an
isolated deterministic RF medium; it deliberately does not connect firmware to
a host network or claim physical-air fidelity.

RP2040 and RP2350 PWM blocks are mapped at their native addresses and use named
register IDs. The functional model covers per-slice CSR/DIV/CTR/CC/TOP
registers, channel-enable aliases, compare outputs, self-clearing phase
advance/retard commands, wrap interrupt status, write-one-to-clear raw status,
and both RP2350 interrupt banks. It uses the
RP2040 eight-slice global layout (`EN` at `0xa0`) and RP2350's twelve-slice
layout (`EN` at `0xf0`, with the second IRQ bank through `0x10c`). Abstract time
advances enabled counters deterministically; exact divider and phase-correct
edge timing, GPIO pin muxing, DMA pacing, and interrupt controller delivery
remain outside this functional slice. The register layout is checked against
the official [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
and [RP2350 datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf).

The ESP32-C6 and ESP32-S3 USB Serial/JTAG models expose a deterministic host
connection control surface. They start connected for existing console fixtures;
tests can call `set_usb_host_connected(false)` to exercise a disconnected,
non-blocking console. A connected host asserts `INT_RAW` SOF bit 1 every 1,000
abstract ticks, and `INT_CLR`/raw writes clear the latched status. This is a
functional USB-frame model, not a claim of USB clock or PHY accuracy. The C6
model additionally exposes the documented CONF0/TEST raw-PHY controls, host
line injection and transition timestamps. Its low-speed packet oracle checks
NRZI, stuffing, PID, CRC16, EOP and firmware instruction cadence without
claiming analog-pad or bus-arbitration fidelity. Main-watchdog system resets
perform the functional flash/application handoff again while preserving the
LP-AON stores, allowing the C6 USB recovery window to be qualified end to end.

The ESP32-S3 DWC2 device link has a descriptor-driven deterministic host for
CDC ACM, HID, CDC Ethernet, MIDI, Audio, WebUSB/vendor, MTP, ADB and MSC. The
qualification firmware performs class-specific control and data transactions,
including a 192-byte isochronous audio packet and MSC BOT INQUIRY/CSW; this is
functional controller qualification rather than electrical USB timing.

The ESP32-C6 machine maps every public non-radio register block in the ESP-IDF
v6.0.2 inventory at its vendor address. GPIO/IO MUX; UART0/1 and LP UART; UHCI;
SPI2; I2C0 and LP I2C; I2S; RMT; LEDC; MCPWM; PCNT; PARLIO; TWAI0/1; SDIO
slave; timer groups and system timer; GDMA; SAR ADC/temperature; ETM; staged
watchdogs; PMU, LP-AON and LP timer; EXTMEM cache maintenance; machine/user PLIC
and CLINT; and the AES, SHA, RSA, ECC, HMAC, digital-signature and eFuse blocks
have deterministic functional slices. The RV32IMAC LP core executes from
retained LP SRAM and participates in modeled sleep, wake and reset transitions.
Remaining analog, trace, debug and monitor/APM blocks are strict target-specific
register facades where no behavioral contract is claimed. ESP32-C6 Wi-Fi,
Bluetooth LE, and IEEE 802.15.4 now use a shared deterministic RF medium with
native MMIO, DMA/shared-memory, and interrupt paths. The IEEE 802.15.4
qualification independently executes freestanding firmware and genuine
ESP-IDF public APIs, requiring exact TX, FCS-bearing RX with RSSI/LQI metadata,
energy detection, interrupt clearing, and byte-identical replay. This is
functional LLE coverage, not an analog PHY or host-network bridge. See
`docs/ESP32C6_PERIPHERALS.md` for the fidelity matrix and omissions.

ESP32-C6 and ESP32-S3 genuine BLE-controller qualification covers hopped
connections, LL version/feature/data-length procedures, ACL/L2CAP ATT traffic,
remote PHY request/response and instant-based bidirectional 1M-to-2M updates,
the public PHY-complete callback, remote termination, scan restart, and exact
deterministic replay. Impossible or overlapping PHY instants stop through the
radio legality validator rather than being silently accepted.

## Implemented CPU surface

The RISC-V interpreter covers RV32I/E integer execution, common compressed
instructions, M and A where selected by the profile, Zicsr, WFI/MRET, basic
machine interrupt entry, QingKe PFIC table-mode entry, all eight QingKe XW
compressed byte/halfword memory operations, the V2C multiply-only Zmmul
subset, and the compiler-facing Hazard3 bit-manipulation subset. Exact PFIC
nesting/HPE remains unsupported. ESP32-C6 supports machine/user transitions,
user ECALL, illegal privileged-CSR traps, delegated interrupt entry, MRET/URET,
and TOR/NA4/NAPOT PMP permission enforcement with architectural access faults.
`qualification/riscv-cpu.json` contains
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

ESP32-S3 DRAM and IRAM power on with the deterministic nonzero byte pattern
`0xa5`. Direct ELF loading copies only each segment's file-backed bytes, so
the synthesized `.bss` tail remains poisoned until firmware clears it. This
keeps direct ELF tests from accidentally depending on loader-provided zeroing.

Verified-image handoff starts with IROM instruction fetch disabled; the ROM
`rom_config_instruction_cache_mode(0x4000, 8, 32)` entry point enables it and
publishes the corresponding `CACHE_STATE` value. A fetch before that call
stops with a diagnostic instead of appearing to work through a coherent-cache
shortcut.

Verified ESP32-S3 application images enter through a modeled `CALLX8`
windowed-ABI handoff. The first application instruction must be `ENTRY`; the
emulator reports the pending `PS.CALLINC` and window depth when that prologue
is absent. Direct ELF loading remains the intentionally weaker debugging mode
with synthetic direct state.

## Timing and tracing

One completed instruction or architectural action advances one abstract tick.
Timers, PIO and external pin stimuli use that deterministic timeline. The
model is not tied to target clock frequency. Baseline PIO executes one
instruction per abstract tick and intentionally ignores divider and delay
timing. VCD uses one nanosecond per abstract tick as a display convention, not
a hardware timing claim.

The ESP32-S3 eFuse slice retains the native staging, read-data, error, timing,
command, status, and interrupt registers. `Esp32S3EfuseRegister` names every
implemented native register and applies the official read/write masks, reset
defaults, reserved-hole rejection, strict aligned 32-bit access, and
read-only/write-only/W1C semantics. It provides deterministic factory identity
words, read-only OTP views, one-way bitwise programming for the documented
blocks, including the native `BLOCK0` staging split between `PGM_DATA0` and
`PGM_DATA1..5`. Programming and read command strobes require the documented
`0x5A5A`/`0x5AA5` opcodes, clear staging registers after programming, redact
key blocks through `RD_DIS`, and expose read/program completion interrupts. It
intentionally does not model programming voltage, Reed-Solomon correction
timing, secure-boot policy, or physical-fuse failure characteristics.

Signals use `0`, `1`, high impedance, and unknown/contention states. Changes
are streamed, and declaration/change digests are stable for equivalent runs.
The CLI accepts scheduled input in `PIN=VALUE@TICK` form.

The ESP32-S3 RMT slice maps transmitter channels 0–3 and receiver channels 4–7
at the native `0x6001_6000` block. Firmware can write the APB FIFO item format,
start a transmit channel, receive a completion interrupt, and observe the
deterministic pulse stream at `board.esp32s3.rmt.ch0` through `ch3` in VCD.
Bounded host-injected receive pulses populate the native receive memory and
interrupt state. Carrier modulation, DMA and source-clock fidelity remain
outside this functional model.

Direct runs accept repeatable `--breakpoint ADDRESS` and `--watchpoint ADDRESS`
controls plus `--stop-signal PATH=change|rising|falling`. Addresses may be
decimal or `0x`-prefixed hexadecimal. A breakpoint stops before executing the
named address. A watchpoint stops after a completed CPU data read/write that
overlaps the named byte, and records the access address and kind in JSON.
Signal stops use stable hierarchical paths and preserve the triggering change
in VCD/digest output. `scripts/qualify-stop-conditions.sh` checks every stop
class on RISC-V, Arm, and Xtensa.

The ESP32-S3 AES accelerator is mapped at `0x6003_a000` using the native
`hwcrypto_reg.h` offsets. The functional slice supports AES-128 and AES-256
single-block encryption/decryption through the text-in/text-out window,
completion busy/interrupt state, and the VCD path
`board.esp32s3.aes.text_out`. DMA cipher modes, GCM/CTR/CBC/CFB/OFB
chaining, eFuse-backed keys, and cycle timing remain unsupported. The model
exposes a named `Esp32S3AesRegister` enum for the complete native key/text,
mode, IV/GCM, DMA, interrupt, and DATE map, applies conservative access masks,
treats trigger/continue/interrupt-clear fields as write-only strobes, and
rejects reserved holes, read-only output writes, and values wider than 32 bits.

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

The ESP32-S3 digital-signature slice follows the native page layout: a
contiguous 396-word C/Y/M/RB/BOX write-only window, four-word IV window, and
128-word X write-only and Z read-only windows, followed by the SetStart,
SetMe, SetFinish, QueryBusy, QueryKeyWrong, QueryCheck, and Date registers.
`Esp32S3DigitalSignatureRegister` enforces the documented masks, reset date,
reserved holes, aligned 32-bit accesses, and access directions. The functional
operation uses a deterministic SHA-256 baseline for protocol testing; secure
key provisioning, RSA-PSS padding and hardware timing remain outside this
qualification slice. `QueryKeyWrong` is reserved for the HMAC-to-DS key
handoff: malformed encrypted parameters set the MD check bit instead. The
model can select ready, not-activated, or failed HMAC handoff states for
deterministic tests. SetFinish clears the native data windows as documented.

The final distillation gate adds four bounded capabilities:

- `remu corpus reduce` minimizes source fragments, flags and numeric inputs,
  recompiling every predicate inside the pinned Docker toolchain. The checked
  three-family evidence is `qualification/reduction.json`.
- `remu script` evaluates Starlark assertions over caller-selected JSON only.
  Its portfolio proof is `qualification/starlark.json`.
- ESP32-C6/S3 direct runs can opt into `--agent-script`. Its `main()` receives
  an opaque live-machine capability for bounded run slices, debugger reads and
  writes, stops, deterministic input, and paginated RF evidence. Scripts may
  load workspace-confined `.star` workflow modules, and `--agent-artifact`
  records their JSON-compatible decisions plus a compact run summary. The C6
  real-ROM gate uses this surface to prove genuine HE20/iTWT timer re-arming
  and repeated wake events over 60 million instructions. The separate
  `--radio-script` callback remains machine-isolated. Neither surface offers
  symbol hooks or substitutes Starlark behavior for an LLE peripheral; see
  `docs/AGENT_STARLARK.md`.
- `remu board` resolves loaded Starlark board definitions into immutable
  topology/actions, then hands that scenario to the Rust board runner. It does
  not expose live CPU, scheduler, or peripheral state to the Starlark VM. With
  `--elf`, the M5StickS3 scenario binds native ESP32-S3 GPIO, SPI3, I2C1,
  I2S0/I2S1 and RMT activity to the complete published non-radio board graph:
  ST7789, M5PM1, BMI270, ES8311 audio, buttons, infrared, Grove and Hat2. See
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
- Espressif ESP32-S3 and ESP32-C6 datasheets and technical reference manuals,
  including the official [ESP32-S3 RMT register definitions](https://raw.githubusercontent.com/espressif/esp-idf/master/components/soc/esp32s3/register/soc/rmt_reg.h)
- Espressif’s official tool package index and crosstool-NG releases

Register behavior not covered by a passing firmware proof remains either
unmapped or explicitly approximate.

The generated per-chip register evidence lives in
`qualification/register-coverage/`. `scripts/docker-smoke.sh` records complete
bus logs, verifies the portfolio, and regenerates all six manifests. The
manifests list observed register addresses and access kinds, proof hashes, and
known functional deviations; an unlisted address is not implicitly claimed as
either supported or unsupported.

The RP2040 and RP2350 ADC blocks are mapped at `0x4004c000` and `0x400a0000`.
They implement the named native register map, deterministic channel selection,
host-provided 12-bit samples, temperature-sensor enable, ready/result,
round-robin selection, an eight-entry FIFO, divider, FIFO-level interrupt
status, and the optional per-sample conversion-error flag in `FIFO[15]`.
The five-channel package model covers RP2040 and RP2350 QFN-60; the device
crate also exposes a nine-channel RP2350 QFN-80 variant. Analog noise,
conversion latency, calibration trim, DMA pacing, and package pin coupling are
intentionally outside this functional CI slice. The register contract is based
on the official [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
and [RP2350 datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf).

Direct-run `--bus-log` output is streamed as an ordered JSON array, so its
memory use is bounded independently of the number of accesses. The schema and
pretty-printed ordering remain compatible with existing qualification
artifacts. Repeated `--bus-log-region NAME` arguments retain only accesses whose
exact bus-region name matches, which keeps long vendor-firmware MMIO evidence
compact without changing execution. Coverage uses the same event stream but is
not affected by that filter and retains all unique executed addresses.
Each RISC-V CPU access includes the causative instruction `pc`. Accesses made
by autonomous peripherals, DMA helpers, debuggers, or host orchestration omit
that field rather than inheriting a stale PC. Direct-memory writes additionally
include safe `pre_value` and `post_value` fields. Device writes omit them unless
the model opts into a side-effect-free internal snapshot; the tracer never
issues a second, potentially side-effecting MMIO read to discover an old value.
The proven C6 RF/PHY register banks provide that snapshot. These fields are
observational and cannot affect bus or device behavior.
`--interrupt-log PATH` independently streams source-level transitions as
timestamped JSON. C6 and S3 radio sources use their native interrupt-matrix
numbers; only level changes are emitted, and autonomous transitions omit `pc`
instead of inheriting the preceding instruction. The exact qualified source,
W1C, reset, and unresolved-private-source boundary is documented in
[`esp-radio-interrupts.md`](esp-radio-interrupts.md) and enforced by
`qualification/radio/interrupt-contract.json`.

For ESP32-C6, direct ELF loading proves instruction and peripheral behavior but
does not exercise the second-stage bootloader's flash mappings. Supplying the
corresponding esptool application binary with `--esp-app-image` enables a
separate boot-layout gate. It checks the chip and entry metadata, descriptor
and text segment ordering, 64 KiB mapping congruence, and correspondence with
the executable ELF. The default application partition offset is `0x10000` and
can be changed with `--esp-app-offset`. This mode requires `--boot-rom` with the
matching real mask-ROM ELF. ESP32-C6/S3 native firmware boot and radio-capable
direct execution have the same requirement; a CPU/compiler-only direct ELF
harness remains exempt.

In direct C6 runs, a main-watchdog system reset reloads the initialized ELF
segments and entry point to model the second-stage application handoff. This
lets reset/recovery firmware run across multiple boots without claiming ROM
instruction-level fidelity.

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
