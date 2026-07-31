# Renvo

Renvo is a deterministic, interpreter-first microcontroller emulation framework
written in Rust. The initial target portfolio is:

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
RENVO_ACCEPTANCE_MAX_SECONDS=60 scripts/qualify-micropython.sh
```

An exhaustive stress mode reruns both the comprehensive and system scenarios
and compares the resulting evidence directly:

```sh
RENVO_CLEAN_REPEATS=2 RENVO_SYSTEM_REPEATS=2 scripts/qualify-micropython.sh
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

Renvo itself builds with the Rust toolchain pinned by `rust-toolchain.toml`.
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
cargo run -p renvo-cli -- targets
```

Build the compiler image once, then compile firmware only through the isolated
corpus runner:

```sh
docker build --pull=false -t renvo/cross-gcc:local toolchains/cross-gcc
cargo run -p renvo-cli -- corpus build \
  --toolchain toolchains/riscv-gcc-rv32ec.toml \
  --source corpus/smoke/riscv \
  --output .renvo/smoke-rv32ec \
  --target ch32v003 \
  --artifact .renvo/smoke-rv32ec-build.json \
  -- -O2 -Wl,-T,link.ld -o /workspace/out/smoke.elf start.S main.c
```

The toolchain TOML must name the immutable image ID produced locally. Corpus
containers run with no network, no capabilities, a read-only root filesystem,
and explicit resource limits. Renvo currently executes compiler smoke ELFs for
CH32V003, CH32V006, ESP32-C6, RP2350 Hazard3, RP2040 Cortex-M0+, and RP2350
Cortex-M33. These are direct-load functional baselines; the target manifests
state the incomplete ISA, interrupt, boot, and peripheral surfaces explicitly.
ESP32-S3 Xtensa LX7 is also runnable with the separately pinned official
Espressif compiler image.

Run an ELF and produce both JSON and VCD:

```sh
cargo run -p renvo-cli -- run \
  --target ch32v003 \
  --elf .renvo/smoke-rv32ec/smoke.elf \
  --max-instructions 100000 \
  --vcd .renvo/smoke-rv32ec/pins.vcd \
  --result .renvo/smoke-rv32ec/run.json
```

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
