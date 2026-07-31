# Containerized firmware toolchains

Renvo never invokes host firmware compilers. Each corpus case names a
`ToolchainSpec` with an immutable Docker digest. The image must already exist
locally; builds run with `--pull=never` and `--network=none`.

The runner mounts:

- the case source at `/workspace/src` read-only;
- a dedicated artifact directory at `/workspace/out` read-write;
- an ephemeral, size-limited `/tmp`.

Images should provide only the compiler, linker, binary utilities, and runtime
files required by their target. `cross-gcc/Dockerfile` pins the RISC-V and Arm
bare-metal GCC compilers. `cross-llvm/Dockerfile` pins Clang 18 and LLD 18 for an
independent Arm/RISC-V code-generation lane, with the GCC packages retained only
to provide target `libgcc` arithmetic helpers. A toolchain TOML records the
resulting immutable image ID; tags are never accepted by the corpus runner.

Build the local image, inspect its immutable ID, and place that ID in the
toolchain TOML:

```sh
docker build --pull=false -t renvo/cross-gcc:local toolchains/cross-gcc
docker image inspect --format '{{.Id}}' renvo/cross-gcc:local
docker build --pull=false -t renvo/cross-llvm:local toolchains/cross-llvm
docker image inspect --format '{{.Id}}' renvo/cross-llvm:local
docker build --pull=false -t renvo/rust-baremetal:local toolchains/rust-baremetal
docker image inspect --format '{{.Id}}' renvo/rust-baremetal:local
```

The build itself may use the package network. Every corpus compilation runs
later with `--pull=never`, `--network=none`, a read-only root filesystem,
dropped capabilities, and explicit CPU, memory, process, and wall-time limits.

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
