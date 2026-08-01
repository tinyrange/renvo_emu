# Portable C edge-case corpus

This directory contains exactly 1,000 deterministic, freestanding C translation
units. Every translation unit has a distinct, mechanically checked control/data
flow signature. The 40 purpose-built embedded kernels are ingredients; each is
combined with a unique 12-operation defined-behavior program, so these are not
input-only variants. Each case returns one `uint32_t`-equivalent result from
volatile inputs. `manifest.tsv` records a descriptive name, category, structural
signature, independently calculated expected result, and upstream inspiration.

The cases deliberately use only portable ISO C unsigned behavior. In particular,
they do not depend on signed overflow, invalid shifts, division by zero,
out-of-bounds access, object aliasing, uninitialized storage, implementation
defined bit fields, or a hosted C library.

The 40 kernels cover:

- CRC-32, CRC-16, CRC-8, Adler, Fletcher, FNV, framing, and COBS;
- sorting, searching, matrices, histograms, ring buffers, and overlap-safe moves;
- popcount, parity, bit reversal, Gray code, register fields, and endian assembly;
- dense and sparse switches, state machines, nested exits, and short-circuit effects;
- eight-argument calls, aggregates by value and return, recursion, and indirect calls;
- timer wraparound, GPIO debounce, UART/SPI-style shifting, and register saturation;
- integer square root and division, fixed-point multiply-accumulate, FIR, and median filters.

The sources are original distilled kernels rather than copied tests. Their design
is informed by [GCC torture tests][gcc-torture], the [LLVM test-suite][llvm-tests],
[Embench-IoT][embench] workloads, and [Csmith][csmith]-style defined-behavior
differential testing. The `inspiration` field preserves that provenance.

The generator encodes the case ID into the first three operations of each
signature and rejects duplicate signatures. Regeneration therefore fails rather
than silently producing fewer than 1,000 structurally distinct cases.

Do not edit generated case files by hand. Regenerate them with:

```sh
cargo run -p remu-casegen -- corpus/edge_cases
```

Firmware compilation is Docker-only. The batch recipes use immutable images,
disable networking, mount this source tree read-only, and record input/output
hashes plus compiler provenance. Run one complete optimization matrix with:

```sh
scripts/edge-corpus.sh -O2 gcc
scripts/edge-corpus.sh -O2 clang
```

Supported optimization values are `-O0`, `-O2`, and `-Os`. GCC builds five ELF
layouts and runs seven execution combinations:

- CH32V003 and CH32V006 from the RV32EC WCH layout;
- RP2350 Hazard3 from the RV32IMAC RP layout;
- ESP32-C6 from the RV32IMAC Espressif layout;
- RP2040 and RP2350 Arm mode from the Cortex-M0+ layout;
- ESP32-S3 from the Xtensa LX7 call0 layout.

Clang 18 with LLD builds the four Arm/RISC-V layouts and runs the first six
combinations. ESP32-S3 remains on Espressif's Xtensa GCC because upstream Clang
does not expose an Xtensa target. Both compiler families use GCC's target
`libgcc` only for freestanding arithmetic helpers.

Failures are collected in stable `remu.corpus-suite.v1` JSON artifacts under
`.remu/`; successful cases are summarized rather than emitted individually.

[gcc-torture]: https://gcc.gnu.org/onlinedocs/gccint/Testsuites.html#index-gcc_002ec_002dtorture
[llvm-tests]: https://llvm.org/docs/TestingGuide.html
[embench]: https://github.com/embench/embench-iot
[csmith]: https://github.com/csmith-project/csmith
