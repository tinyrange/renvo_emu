# Renvo Emulator

[![CI](https://github.com/tinyrange/renvo_emu/actions/workflows/ci.yml/badge.svg)](https://github.com/tinyrange/renvo_emu/actions/workflows/ci.yml)

**A deterministic MCU firmware CI and compiler-validation engine with
evidence-backed target models.**

Renvo Emulator is an interpreter-first microcontroller emulation framework
written in Rust. The command-line tool and crate namespace use the short name
`remu`. This is a separate project from the lightweight
[Renvo Go compiler](https://github.com/tinyrange/renvo).

The project is built for reproducible, bounded automation: run compiler output
or deployable firmware images, inject external signals, stop on architectural or
electrical events, and retain machine-readable evidence. Its target portfolio
includes mainstream parts alongside QingKe, MSP430, PIC16, AVR, and MCS-51
devices that are often missing from general-purpose emulators.

Renvo Emulator does **not** currently claim cycle accuracy, complete ISA
coverage, complete peripheral coverage, physical RF accuracy, or arbitrary SDK
compatibility. One abstract tick represents a completed instruction or
architectural action. That timing is deterministic and useful for ordering,
timers, and waveforms, but it is not silicon time.

## Why remu?

- **Deterministic by construction.** Stable event ordering, explicit execution
  limits, canonical result and signal digests, and deterministic replay make
  failures suitable for CI and reduction.
- **Firmware artifacts are first-class.** ELF, UF2, Espressif flash images,
  Intel HEX, and raw binaries can exercise the appropriate direct-load or
  reset/boot boundary.
- **Evidence accompanies compatibility claims.** Qualification records compiler
  containers, commands, source and firmware hashes, register coverage,
  provenance, repeated results, VCD, and known deviations.
- **Observability is headless and scriptable.** JSON results, UART and USB
  transcripts, VCD, bus logs, coverage, breakpoints, watchpoints, named signal
  stops, Starlark assertions, and GDB remote debugging are available without a
  GUI.
- **Toolchains are isolated.** Firmware test cases are compiled in immutable,
  network-disabled Docker containers rather than through ambient host
  compilers.

## Support tiers

“Supported” is deliberately split into three cumulative tiers:

1. **Compiler execution** — compiler-produced ELF runs through a documented CPU
   and ABI subset with bounded results and tracing.
2. **Firmware functional slice** — native reset/image handling and a tested
   subset of chip-specific GPIO, timer, UART, interrupt, and startup behavior.
3. **Selected board or SDK workflow** — a named vendor sample, SDK path, or
   official firmware image reaches a tested endpoint.

Tier 3 never means that arbitrary firmware for the chip will work. It means
only that the exact published workflows pass. The authoritative implemented
and deferred behavior is in [the support contract](docs/SUPPORT.md), while the
generated [qualification dashboard](qualification/dashboard.html) binds claims
to checked evidence.

## Quick start

You need Git, a current stable Rust toolchain, and a Linux host. Docker is only
required when compiling the supplied firmware corpus or regenerating the
qualification evidence.

```sh
git clone https://github.com/tinyrange/renvo_emu.git
cd renvo_emu
cargo build --release -p remu-cli
./target/release/remu targets
```

Run a deployable RP2040 UF2 and retain both a structured result and waveform:

```sh
./target/release/remu firmware boot \
  --target rp2040 \
  --image firmware/blink.uf2 \
  --max-instructions 1000000 \
  --result build/blink-result.json \
  --vcd build/blink.vcd
```

The JSON result contains the target, bounded stop reason, abstract time and
instruction counts, named CPU state, UART output, and a deterministic trace
digest. The VCD can be opened with GTKWave or another standard waveform viewer.
Long-running firmware should use an explicit signal, breakpoint, deadline, or
instruction limit as its stopping contract.

Renvo can also run in a browser as a WASI Preview 2 component. The component
exports a versioned WIT API for target discovery, ELF inspection, ELF execution,
and Intel HEX execution; `jco` turns that component into browser-loadable ES
modules. See [the WebAssembly guide](docs/WEBASSEMBLY.md) and the runnable
example in [`web/`](web/).

For compiler and ABI work, run an ELF directly:

```sh
./target/release/remu run \
  --target ch32v003 \
  --elf build/firmware.elf \
  --max-instructions 100000 \
  --result build/run.json \
  --vcd build/pins.vcd
```

Direct ESP execution is an architectural/compiler oracle, not proof that a
bootloader accepts the flash layout. Supplying `--esp-app-image` adds a separate
esptool-compatible application-image validation step and requires `--boot-rom`
with the target's real mask-ROM ELF. ESP32-C6/S3 native firmware boot and
radio-capable direct runs likewise require the matching ROM. A direct ELF run
that only tests CPU/compiler behavior remains exempt.

The ESP32-S3 model also exposes a functional RMT slice. Transmit channels 0–3
accept native FIFO pulse items at `0x6001_6000`, set completion status, and
publish deterministic channel waveforms to VCD as
`board.esp32s3.rmt.ch0`–`ch3`. Receive channels 4–7 accept bounded host-injected
pulse items through their native memory windows and interrupt state. Carrier
modulation, DMA and clock-accurate behavior are not claimed.

Tagged releases publish Linux/amd64 and Linux/arm64 binaries, checksums, and a
runtime-only OCI image. The image contains no compiler toolchains and runs as a
non-root user. For a source-level example, see
[`examples/quickstart`](examples/quickstart/README.md).

```sh
docker run --rm -v "$PWD:/work:ro" -w /work \
  ghcr.io/tinyrange/renvo_emu:latest targets
```

The image is for running existing ELF/native firmware artifacts. Compilation
of corpus cases remains isolated in the pinned Docker toolchain images.

## Target portfolio

The repository contains 13 MCU models and 14 execution modes because RP2350 is
qualified in both Arm and RISC-V configurations.

| MCU model | CPU mode(s) | Highest demonstrated tier | Native image |
| --- | --- | --- | --- |
| WCH CH32V003 | QingKe V2 RV32EC | 3 — selected WCH EVT | raw binary, Intel HEX |
| WCH CH32V006 | QingKe V2 RV32EC | 3 — selected WCH EVT | raw binary, Intel HEX |
| Raspberry Pi RP2040 | Cortex-M0+ | 3 — Pico SDK and official MicroPython | UF2 |
| Raspberry Pi RP2350 | Cortex-M33, Hazard3 RV32IMAC | 3 — Pico SDK and official MicroPython | UF2 |
| Espressif ESP32-S3 | Xtensa LX7 | 3 — ESP-IDF sample and official MicroPython | merged flash binary, application UF2 overlay |
| Espressif ESP32-C6 | RV32IMAC | 3 — ESP-IDF sample and official MicroPython | merged flash binary, application UF2 overlay |
| Microchip ATSAMD21E18A | Cortex-M0+ | 2 — firmware functional slice | raw binary, Intel HEX |
| STMicroelectronics STM32L432KC | Cortex-M4F | 2 — firmware functional slice | raw binary, Intel HEX |
| Renesas R7FA4M1AB3CFM | Cortex-M4F | 2 — firmware functional slice | raw binary, Intel HEX |
| Microchip ATmega328PB | enhanced AVR8 | 2 — firmware functional slice | Intel HEX, raw binary |
| Texas Instruments MSP430FR2433 | MSP430Xv2 | 2 — firmware functional slice | Intel HEX, raw binary |
| Microchip PIC16F15376 | enhanced mid-range PIC16 | 2 — firmware functional slice | Intel HEX |
| Silicon Labs EFM8BB52F32G | CIP-51/MCS-51 | 2 — firmware functional slice | Intel HEX |

Each target has a public peripheral checklist in the repository’s
[GitHub issues](https://github.com/tinyrange/renvo_emu/issues). An unchecked
peripheral may have address space reserved for startup compatibility, but it
has no supported behavioral model until deterministic tests and documentation
say otherwise.

The ESP32-S3 model includes deterministic native I2C0 and I2C1 command/FIFO
transactions, attachable board devices, and SDA/SCL waveform signals. The
qualified catalogue includes SGP30, M5PM1, BMI270 and ES8311 behavior. This is a
functional peripheral slice, not a clock-accurate electrical bus model.

The ESP32-S3 functional slice includes native-address SPI2 and SPI3 user
transactions: firmware can fill the W0-W15 FIFO, start a bounded transfer, and
observe deterministic MOSI, MISO, SCLK, CS0, and transfer-complete signals. The
current general-purpose model intentionally leaves DMA, clock-divider fidelity,
and additional chip-select electrical behavior for later peripheral slices. The
M5StickS3 attachment couples SPI3 to a functional ST7789 framebuffer.

The ESP32-S3 I2S functional slice supports native I2S0/I2S1 register setup,
single-data stereo-frame starts, TX/RX completion status, loopback samples, and
deterministic MCLK/BCLK/WS/DOUT/DIN VCD signals. DMA audio streaming, PDM/TDM
transforms, and clock-frequency fidelity remain outside this bounded model.

The selected M5StickS3 workflow couples a vendor-toolchain-built Xtensa ELF to
the complete published non-radio board graph: ST7789 display and backlight,
M5PM1 power/IRQ/telemetry, BMI270 IMU, ES8311 microphone and speaker paths,
buttons, bidirectional infrared, Grove and Hat2 pins. Native GPIO, SPI3, I2C1,
I2S0/I2S1 and RMT register activity updates one deterministic board snapshot
and VCD. Radio behavior is intentionally tracked separately. Run a built ELF
with:

```sh
remu board --file qualification/m5sticks3-live.star --elf FILE
```

## Qualification and provenance

The checked qualification suite covers more than workspace unit tests:

- a 1,000-case UB-free C corpus across GCC, Clang, optimisation levels, and
  seven initial CPU/target combinations;
- exact-RV32E, RV32IMAC, Armv6-M, and Armv8-M Rust ABI workloads;
- selected unmodified WCH EVT, Pico SDK, and ESP-IDF source samples pinned to
  immutable upstream commits;
- a pinned RP2040 Pico SDK UART/PWM regression slice with native register
  evidence in `qualification/rp2040-sdk.json`;
- a pinned RP2040 Pico SDK multicore FIFO/launch regression with shared-UART
  evidence in `qualification/rp2040-multicore.json`;
- an explicit RP2040/RP2350 implementation and gap matrix in
  `docs/RASPBERRY_PI_STATUS.md`;
- native-image versus direct-execution equivalence for all 14 target modes;
- architectural stop conditions, GDB, coverage, replay, bus logs, VCD, and
  register-coverage generation;
- EEMBC CoreMark correctness plus host-calibrated interpreter throughput;
- host-determinism evidence on Linux/amd64 and Linux/arm64; and
- official MicroPython v1.28.0 on M5Stack NanoC6, M5Stack AtomS3 Lite,
  Raspberry Pi Pico, and Pico 2 in both CPU modes.

The MicroPython gate checks portable runtime behavior, raw-REPL recovery,
threads, timers, externally driven GPIO, persistent storage, deterministic VCD,
and the pinned MQuickJS cross-runtime workload. It does not imply complete
MicroPython, radio, ADC, PWM, watchdog, or serial-bus compatibility.

Useful entry points are:

```sh
# Initial six-target compiler, peripheral, and provenance gate
scripts/docker-smoke.sh

# Pinned upstream RP2040 SDK UART/PWM regression slice
scripts/qualify-rp2040-sdk.sh

# Pinned upstream RP2040 SDK multicore FIFO regression slice
scripts/qualify-rp2040-multicore.sh

# All 13 MCUs, including native-image equivalence
scripts/qualify-expansion.sh

# Official firmware acceptance; warm-cache budget is one minute
REMU_ACCEPTANCE_MAX_SECONDS=60 scripts/qualify-micropython.sh

# CoreMark correctness and host throughput
COREMARK_OFFLINE=1 scripts/qualify-coremark.sh

# Cross-host deterministic scheduler evidence
scripts/qualify-host-determinism.sh
```

The default GitHub Actions workflow currently runs formatting, Clippy, all
workspace tests, source-layout checks, and package-manifest validation. The
larger Docker, native-image, MicroPython, CoreMark, and host-determinism gates
remain explicit qualification commands with checked artifacts in the
repository; moving reproducible subsets into scheduled public CI is follow-up
work.

Read [CoreMark methodology and results](docs/COREMARK.md), the
[1,000-case corpus notes](corpus/edge_cases/README.md), and the
[restored-plan status ledger](docs/PLAN_STATUS.md) for the detailed evidence.

## Docker-only compiler corpus

Bootstrap the reviewed local toolchain images, then compile through the
contained corpus runner:

```sh
scripts/bootstrap-toolchains.sh

cargo run -p remu-cli -- corpus build \
  --toolchain toolchains/riscv-gcc-rv32ec.toml \
  --source corpus/smoke/riscv \
  --output .remu/smoke-rv32ec \
  --target ch32v003 \
  --artifact .remu/smoke-rv32ec-build.json \
  -- -O2 -Wl,-T,link.ld -o /workspace/out/smoke.elf start.S main.c
```

Toolchain manifests bind a reviewed image identity and a locally buildable
fallback tag. Builds use no network, a read-only root filesystem, dropped
capabilities, `no-new-privileges`, dedicated mounts, resource limits, stable
environment ordering, and complete input/output hashes.

The same infrastructure can compare compiler configurations and reduce a
divergence across source fragments, flags, linked runtime, and numeric inputs.

## Stops, traces, and debugging

Direct runs support:

- instruction and virtual-time limits;
- architectural exit, fault, and breakpoint stops;
- address breakpoints and overlapping data watchpoints;
- named `change`, `rising`, and `falling` signal stops;
- scheduled external pin drives in `PIN=VALUE@TICK` form;
- streaming VCD and bounded-memory bus logs;
- symbolicated instruction coverage and deterministic replay; and
- one-session GDB remote debugging.

Signal values include logic low, logic high, high impedance, and
unknown/contention. VCD’s one-nanosecond timescale is a display convention for
abstract ticks, not a hardware frequency claim.

## Starlark boards and connected devices

Starlark scripts describe board topology and bounded test actions while Rust
owns all state and timing. The initial M5Stack NanoC6 definition exposes its
button, blue LED, WS2812, and Grove connector. Reusable Rust components include
push buttons, LEDs, WS2812 RGB LEDs, and an SGP30 sensor; a test can attach the
sensor with `board.connect("grove", sensor)`.

The generic board/component scenario validates topology and digital protocol
models independently. Live ESP32-C6 firmware MMIO is not yet routed into its
assembled board graph. The M5StickS3 scenario does bind live ESP32-S3 firmware
to its complete non-radio board graph end to end.
For radio workflows, `remu run --radio-script peer.star` attaches a bounded,
event-driven Starlark peer to the isolated RF medium. Its `on_event(event,
state)` callback can retain explicit JSON state and schedule future RF frames;
it cannot inspect CPU state, memory, registers, symbols, files, or host network
resources. `--radio-repl` enables `repl()`/`breakpoint()` inside that callback,
which is useful for iterating on virtual peers while genuine firmware controls
the low-level radio peripheral.

For agent-driven investigation, `remu run --agent-script drive.star` exposes a
long-lived, opaque ESP32-C6 or ESP32-S3 `machine` value to the script's
`main()` function. Bounded methods resume execution, inspect CPU and memory,
set debug stops, drive GPIO/USB input, inject isolated RF frames, and page
through replay evidence. A filtered bounded bus-capture ring lets the same
live script or REPL inspect native MMIO/DMA sequences without retaining an
unbounded trace, and composes with the existing streaming `--bus-log`. Scripts
can load workspace-confined reusable `.star`
workflows, `--agent-repl` enables a scoped `repl()` call in the same live
session, and `--agent-artifact` records the script result without duplicating
the full UART/RF streams. This driver is an orchestration and diagnostics boundary:
it has no symbol-hook API and does not implement peripheral behavior in
Starlark. Real mask ROM remains mandatory. See
[agent-driven Starlark](docs/AGENT_STARLARK.md).

The RISC-V, Arm RP, and Xtensa machine APIs now expose named `Compiler` and
`Native` UART endpoints for the transport-coupling work that follows; vendor
UART adapters remain target-specific for now.
See [board simulation](docs/BOARD_SIMULATION.md) for the exact boundary.

## Architecture

The workspace keeps CPU interpreters independent from scripting, tracing, and
front-end concerns:

- `remu-core`, `remu-bus`, `remu-signals`, and `remu-trace` provide abstract
  time, events, address spaces, resolved nets, and deterministic artifacts.
  `remu-core::Scheduler` owns monotonic time and typed due-event dispatch over
  the stable event queue; target/device migration is intentionally incremental.
- `remu-machines::RunControl` owns stable stimulus ordering, shared
  instruction/time limits, and signal trace/digest streaming for the RISC-V,
  Arm, and ATmega run paths; target-specific CPU, interrupt, boot, and device
  work remains in each machine implementation.
- `remu-cpu-*` crates implement RISC-V, Arm, Xtensa, AVR, MSP430, PIC16, and
  MCS-51 execution.
- `remu-devices` provides reusable chip and board components.
- `remu-machines` assembles CPU, memory, boot services, interrupts, and
  peripherals into targets.
- `remu-image` handles ELF and deployable firmware formats.
- `remu-corpus`, `remu-starlark`, `remu-gdb`, and `remu-cli` provide the
  automation surface without being embedded in CPU implementations.

The design is interpreter-only today. Published
[CoreMark results](docs/COREMARK.md) measure roughly 9–23 standard emulated
iterations per host second on the documented i7-1165G7 host, depending on the
target. Those are emulator-throughput measurements, never MCU silicon scores.

## Known boundaries

- Functional timing is approximate and deterministic, not cycle or clock
  accurate.
- ISA implementations cover qualified compiler output rather than every
  architectural or undocumented instruction.
- Interrupt priority, nesting, DMA, analog, clock trees, low-power behavior,
  flash controllers, USB fidelity, and radios vary by target and are often
  partial or absent.
- Direct ELF execution may bypass boot-image behavior; use native-image paths
  where the distinction matters.
- Live machine-MMIO coupling currently exists for the M5StickS3 non-radio
  board graph; other Starlark boards remain component/protocol scenarios.
- The interpreter prioritises repeatability and instrumentation over QEMU-class
  throughput; there is no JIT.

These boundaries are model metadata and qualification inputs, not informal
disclaimers. Register behavior without a passing firmware proof remains
unmapped or explicitly approximate.

## Roadmap

The next cross-cutting work is tracked explicitly:

1. [Run representative qualification in public GitHub Actions](https://github.com/tinyrange/renvo_emu/issues/22)
2. [Publish reproducible binaries, an OCI image, and a runnable example](https://github.com/tinyrange/renvo_emu/issues/23)
3. [Centralize deterministic machine scheduling and run control](https://github.com/tinyrange/renvo_emu/issues/24)
4. [Connect Starlark board topology to live firmware MMIO](https://github.com/tinyrange/renvo_emu/issues/25)
5. [Add hardware-backed and differential correctness oracles](https://github.com/tinyrange/renvo_emu/issues/26)
6. [Generate the public capability matrix from qualification evidence](https://github.com/tinyrange/renvo_emu/issues/27)
7. [Deepen RP2040 as the flagship Pico SDK target](https://github.com/tinyrange/renvo_emu/issues/28)
8. [Establish interpreter performance benchmarks and budgets](https://github.com/tinyrange/renvo_emu/issues/29)

Per-target peripheral depth remains tracked separately in the existing MCU
checklist issues.

## GitHub Actions example

A project can build remu from source and retain its result and VCD as CI
artifacts:

```yaml
steps:
  - name: Check out firmware project
    uses: actions/checkout@v4
  - name: Check out Renvo Emulator
    uses: actions/checkout@v4
    with:
      repository: tinyrange/renvo_emu
      path: remu
  - uses: dtolnay/rust-toolchain@stable
  - run: cargo build --release -p remu-cli
    working-directory: remu
  - name: Run firmware
    run: |
      remu/target/release/remu firmware boot \
        --target rp2040 \
        --image firmware/blink.uf2 \
        --result artifacts/run.json \
        --vcd artifacts/pins.vcd
  - uses: actions/upload-artifact@v4
    with:
      name: remu-evidence
      path: artifacts/
```

For a production workflow, pin third-party actions to reviewed commit SHAs and
make the expected stop reason, exit code, output, and trace digest explicit test
assertions.

## Documentation

- [Support contract](docs/SUPPORT.md)
- [Generated support dashboard](qualification/dashboard.html)
- [Expansion target acceptance](docs/EXPANSION.md)
- [Board simulation](docs/BOARD_SIMULATION.md)
- [CoreMark qualification](docs/COREMARK.md)
- [Restored-plan status](docs/PLAN_STATUS.md)
- [Architecture decisions](docs/adr/)
- [Release and runtime image contract](docs/RELEASE.md)
- [Original implementation plan](PLAN.html)

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
scripts/check-source-layout.sh
```

## License

Renvo Emulator is available under either the Apache License 2.0 or the MIT
License, at your option. See `LICENSE`, `LICENSE-APACHE`, and `LICENSE-MIT`.
Inputs fetched by qualification scripts retain their own terms as described in
`THIRD_PARTY_LICENSES.md`.
