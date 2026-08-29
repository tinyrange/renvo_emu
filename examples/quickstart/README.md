# Renvo Emulator quick start

This example is a small CH32V003 GPIO program. It is deliberately kept as
source so the example can be rebuilt with the same Docker-only compiler
workflow as the qualification corpus.

The tagged release archive also contains `build/quickstart.elf`,
`build.json`, `run.json`, `run.vcd`, and `SHA256SUMS`. Those files are generated
by the release workflow from this source and can be run without a compiler:

```sh
docker run --rm -v "$PWD:/work:ro" -w /work \
  ghcr.io/tinyrange/renvo_emu:latest \
  run --target ch32v003 --elf build/quickstart.elf \
  --max-instructions 10000
```

From a checkout, build the ELF with the pinned RISC-V container:

```sh
mkdir -p .remu/quickstart
./target/release/remu corpus build \
  --toolchain toolchains/riscv-gcc-rv32ec.toml \
  --source examples/quickstart \
  --output .remu/quickstart/build \
  --target ch32v003 \
  --artifact .remu/quickstart/build.json \
  -- -O2 start.S main.c -Wl,-T,link.ld -o /workspace/out/quickstart.elf
```

Run it with a locally built binary or the published runtime image:

```sh
remu run --target ch32v003 \
  --elf .remu/quickstart/build/quickstart.elf \
  --max-instructions 10000 \
  --result .remu/quickstart/run.json \
  --vcd .remu/quickstart/run.vcd

docker run --rm -v "$PWD:/work:ro" -w /work \
  ghcr.io/tinyrange/renvo_emu:latest \
  run --target ch32v003 \
  --elf .remu/quickstart/build/quickstart.elf \
  --max-instructions 10000
```

The bounded run terminates with `reason: "Halted"` and `exit_code: 0`. The VCD
contains the native CH32 GPIO signal tree, and the JSON artifact records the
target, instruction/time limits, and deterministic trace digest. The container
is runtime-only: compiler images remain immutable inputs to `corpus build`.
