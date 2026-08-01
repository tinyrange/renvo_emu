# Containerized firmware toolchains

Renvo Emulator never invokes host firmware compilers. Each corpus case names a
`ToolchainSpec` with a reviewed immutable Docker image ID and a locally built
fallback tag. Renvo Emulator resolves either reference to an immutable ID before use;
builds run with `--pull=never` and `--network=none`.

The runner mounts:

- the case source at `/workspace/src` read-only;
- a dedicated artifact directory at `/workspace/out` read-write;
- an ephemeral, size-limited `/tmp`.

Images should provide only the compiler, linker, binary utilities, and runtime
files required by their target. `cross-gcc/Dockerfile` pins the RISC-V and Arm
bare-metal GCC compilers. `cross-llvm/Dockerfile` pins Clang 18 and LLD 18 for an
independent Arm/RISC-V code-generation lane, with the GCC packages retained only
to provide target `libgcc` arithmetic helpers. A toolchain TOML records both
the reviewed image ID and the corresponding local fallback tag.

Build the standard local toolchain set without editing any TOML files:

```sh
scripts/bootstrap-toolchains.sh
```

The PIC/XC8 lane requires explicit acknowledgement after reviewing its vendor
terms: `scripts/bootstrap-toolchains.sh --include-xc8`.

The build itself may use the package network. Every corpus compilation runs
later with `--pull=never`, `--network=none`, a read-only root filesystem,
dropped capabilities, and explicit CPU, memory, process, and wall-time limits.

The seven-target expansion uses target-specific immutable specifications for
ATSAMD21E18, STM32L432KC, R7FA4M1AB3CFM, ATmega328PB, MSP430FR2433,
PIC16F15376, and EFM8BB52F32G. The three Arm targets have both GCC 13.2.Rel1
and Clang/LLD 18 specifications. The AVR, MSP430, XC8, and SDCC Dockerfiles pin
their compiler/device-pack archives and verify published or recorded SHA-256
values where redistribution terms permit local image construction.

Individual target-specific images can also be built directly:

```sh
docker build --pull=false -t remu/avr-gcc:local toolchains/avr-gcc
docker build --pull=false -t remu/msp430-gcc:local toolchains/msp430-gcc
docker build --pull=false -t remu-xc8:4.00-pic16f1xxxx-1.31.465 toolchains/xc8
docker build --pull=false -t remu/sdcc-mcs51:4.5.0 toolchains/sdcc-mcs51
```

The XC8 recipe downloads Microchip's installer for a local qualification
image. Do not publish or redistribute that image; review and accept the vendor
licence yourself. Renvo Emulator records only the recipe, pinned installer/device-pack
hashes, and immutable local image identity.

`xtensa-esp-gcc/Dockerfile` installs Espressif's official Xtensa GCC archive.
Both the release URL and SHA-256 published in Espressif's package index are
pinned, and the archive checksum is verified before extraction.

`rust-baremetal/Dockerfile` pins the official Rust 1.97.1 image and installs
the upstream RV32IMAC, Armv6-M, and Armv8-M bare-metal libraries. Rust exposes
an exact `riscv32e-unknown-none-elf` register-ABI target specification but does
not ship its library, so the image compiles `core` once from its pinned
`rust-src` and installs that artifact into the sysroot. This makes later RV32E
case builds fast and fully offline; the separate GCC lane covers compressed
RV32EC code generation.

`riscv-gcc-hazard3.toml` records the RP2350 software-facing
`rv32imac_zicsr_zifencei_zba_zbb_zbs_zbkb_zcb_zcmp` ISA string. GCC 13 accepts
that complete string and emits a checked assembly artifact. Its bundled
binutils predates the Zcb/Zcmp attribute names, so the executable qualification
assembles the supported compiler output and supplies specification-defined raw
Zcb/Zcmp halfwords in the same CPU harness. The build provenance records and
hashes both artifacts.
