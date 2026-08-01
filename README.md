# Renvo Emulator

Renvo Emulator is a deterministic, interpreter-first microcontroller emulation
framework written in Rust. Its short name, used by the command-line tool and
code namespaces, is `remu`. It is a separate project from the lightweight
[Renvo Go compiler](https://github.com/tinyrange/renvo).

The initial target portfolio is:

- WCH CH32V003
- WCH CH32V006
- Raspberry Pi RP2040
- Raspberry Pi RP2350
- Espressif ESP32-S3
- Espressif ESP32-C6

The implementation follows [PLAN.html](PLAN.html). The first compatibility tier
focuses on compiler-produced ELF execution, architectural exceptions,
interrupts, GPIO, timers, UART, external pin stimulus, and VCD traces. It does
not claim cycle accuracy or complete peripheral coverage.
The exact implemented/deferred boundary is in
[docs/SUPPORT.md](docs/SUPPORT.md).
Progress against the restored plan's own exit gates is tracked in
[docs/PLAN_STATUS.md](docs/PLAN_STATUS.md).
The 1,000-case portable C baseline is documented in
[corpus/edge_cases/README.md](corpus/edge_cases/README.md).
The six-MCU CoreMark qualification and host-calibrated results are in
[docs/COREMARK.md](docs/COREMARK.md).
The generated six-target support, provenance and known-gap dashboard is
[qualification/dashboard.html](qualification/dashboard.html).

## License

Renvo Emulator is available under either the Apache License 2.0 or the MIT License, at
your option. See `LICENSE`, `LICENSE-APACHE`, and `LICENSE-MIT`. Inputs fetched
by qualification scripts retain their own terms as described in
`THIRD_PARTY_LICENSES.md`.

The frozen seven-target expansion adds ATSAMD21E18, STM32L432KC,
R7FA4M1AB3CFM, ATmega328PB, MSP430FR2433, PIC16F15376, and EFM8BB52F32G. Its
exact functional boundary and comprehensive acceptance command are documented
in [docs/EXPANSION.md](docs/EXPANSION.md). Run the complete expansion and
original-six regression gate with:

```sh
scripts/qualify-expansion.sh
```

## Starlark boards and connected devices

Board topology can be defined once in Starlark and loaded by test scripts. The
initial M5Stack NanoC6 model exposes its onboard button, blue LED, WS2812 and
Grove connector; reusable Rust models cover those devices plus the SGP30. A
test attaches the sensor with `board.connect("grove", sensor)` and can emit a
deterministic VCD of the resulting pin and protocol activity.

The API, fidelity boundary, example scenario, and qualification command are in
[docs/BOARD_SIMULATION.md](docs/BOARD_SIMULATION.md).

## Official MicroPython qualification

The current offline qualification boots unmodified official MicroPython
v1.28.0 firmware for M5Stack NanoC6, M5Stack AtomS3 Lite, Raspberry Pi Pico,
and Raspberry Pi Pico 2 in both Arm and RISC-V modes. The release gate runs
every distinct check once and validates the comprehensive results against
fixed per-profile evidence digests:

- a 15-case portable runtime workload
- soft reset and raw-REPL recovery
- multicore/thread behavior
- periodic and one-shot timer callbacks
- externally driven GPIO input and GPIO output helpers
- persistent filesystem write/read across separate boots
- VCD and ordered trace-digest checks
- the pinned MQuickJS cross-runtime workload

Run the complete release gate with:

```sh
scripts/qualify-micropython.sh
```

On an eight-thread host, the gate dispatches independent firmware boots in
parallel. Enforce the one-minute release budget with:

```sh
REMU_ACCEPTANCE_MAX_SECONDS=60 scripts/qualify-micropython.sh
```

An exhaustive stress mode reruns both the comprehensive and system scenarios
and compares the resulting evidence directly:

```sh
REMU_CLEAN_REPEATS=2 REMU_SYSTEM_REPEATS=2 scripts/qualify-micropython.sh
```

Raw-REPL script runs retain their conservative instruction ceilings but stop
once every queued chunk has executed and the matching final prompt is
observed. The release gate performs 25 official-firmware runs plus five pinned
MQuickJS profiles. The latest local v4 evidence completed in 38.882 seconds.

This is additional evidence beyond the restored baseline contract in
[PLAN.html](PLAN.html). A broader MicroPython suite, watchdog/PWM/ADC/serial-bus
coverage, and virtual ESP radios are deliberately not completion requirements
for that plan.

## Toolchain isolation

Renvo Emulator itself builds with the Rust toolchain pinned by `rust-toolchain.toml`.
Firmware corpus cases are compiled only in pinned Docker containers. The corpus
runner records the immutable image ID, command, target, flags, and input hashes
with every build artifact.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

## Current vertical slices

List the source-linked portfolio manifests:

```sh
cargo run -p remu-cli -- targets
```

Bootstrap the local compiler images once, then compile firmware only through
the isolated corpus runner:

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

Toolchain TOMLs record a reviewed image ID and a locally buildable fallback
tag. The runner resolves either one to an immutable ID before execution and
records that ID in the build artifact. Corpus containers run with no network,
no capabilities, a read-only root filesystem, and explicit resource limits.
Renvo Emulator currently executes compiler smoke ELFs for
CH32V003, CH32V006, ESP32-C6, RP2350 Hazard3, RP2040 Cortex-M0+, and RP2350
Cortex-M33. These are direct-load functional baselines; the target manifests
state the incomplete ISA, interrupt, boot, and peripheral surfaces explicitly.
ESP32-S3 Xtensa LX7 is also runnable with the separately pinned official
Espressif compiler image.

Run an ELF and produce both JSON and VCD:

```sh
cargo run -p remu-cli -- run \
  --target ch32v003 \
  --elf .remu/smoke-rv32ec/smoke.elf \
  --max-instructions 100000 \
  --vcd .remu/smoke-rv32ec/pins.vcd \
  --result .remu/smoke-rv32ec/run.json
```

Direct ESP32-C6 ELF execution is an architectural/compiler oracle; it cannot by
itself prove that the vendor bootloader can map the generated flash image. Pass
the application binary produced by `esptool.py elf2image` to check the image
layout against the ELF before execution:

```sh
cargo run -p remu-cli -- run \
  --target esp32c6 \
  --elf build/firmware.elf \
  --esp-app-image build/firmware.bin \
  --esp-app-offset 0x10000
```

The check rejects merged or reordered mapped segments, a missing application
descriptor, invalid 64 KiB flash/virtual alignment, an entry point outside the
mapped text segment, and entry bytes that differ from the ELF. Without
`--esp-app-image`, the CLI emits an explicit warning rather than implying that
direct execution validates flash bootability.

`--bus-log` writes the same deterministic JSON array in execution order while
the emulator runs. It does not retain the complete access history in memory;
coverage retains only its unique instruction-address set.

Every exposed target also accepts its deployable flash artifact through
`remu firmware boot`. Format detection uses container magic for UF2 and ESP
images, a leading `:` for Intel HEX, and otherwise treats the input as a raw
binary rooted at the target's primary flash base.

| Targets | Accepted native image |
| --- | --- |
| CH32V003, CH32V006 | raw binary or Intel HEX |
| RP2040, RP2350 Arm, RP2350 RISC-V | UF2 |
| ESP32-S3, ESP32-C6 | merged Espressif flash binary; application UF2 with `--esp-base-image` |
| ATSAMD21E18, STM32L432KC, R7FA4M1AB3CFM | raw binary or Intel HEX |
| ATmega328PB, MSP430FR2433 | Intel HEX or raw binary |
| PIC16F15376, EFM8BB52F32G | Intel HEX |

For example:

```sh
cargo run -p remu-cli -- firmware boot \
  --target atmega328pb \
  --image build/firmware.hex \
  --result build/native-run.json
```

`scripts/qualify-native-images.sh` compiles probes in the pinned Docker
toolchains, converts them to each target's deployable format, and compares the
native boot path with direct execution. It covers 13 MCUs and 14 target modes
(both RP2350 architectures), including identical stop results and VCD traces,
plus wrong-family, wrong-chip, and malformed-format rejection cases. Its
checked evidence is `qualification/native-images.json`; the comprehensive
under-one-minute expansion gate runs it in parallel with the other suites.

After both Dockerfiles have been built and their immutable IDs match the
toolchain TOMLs, `scripts/docker-smoke.sh` compiles and runs the complete
six-chip portfolio. RP2350 is exercised in both Cortex-M33 and Hazard3 modes.
The gate also runs three-family reduction, GDB/coverage/replay, unmodified
vendor samples, bounded Starlark assertions, and regenerates the support
dashboard. Its two final warm-cache runs each completed in 29 seconds and
produced byte-identical qualification trees.

Run the Docker-only behavioral corpus at one supported optimization level:

```sh
scripts/edge-corpus.sh -O2 gcc
scripts/edge-corpus.sh -O2 clang
```

Run the pinned EEMBC CoreMark correctness and host-throughput qualification:

```sh
COREMARK_OFFLINE=1 scripts/qualify-coremark.sh
```
