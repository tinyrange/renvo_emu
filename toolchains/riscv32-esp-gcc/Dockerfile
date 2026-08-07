FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ADD https://github.com/espressif/crosstool-NG/releases/download/esp-14.2.0_20260121/riscv32-esp-elf-14.2.0_20260121-x86_64-linux-gnu.tar.gz /tmp/riscv32-esp-elf.tar.gz

RUN echo "1b79617f43cf0e25b92646359d1623d8bef135050aca44586df59b6a64496d82  /tmp/riscv32-esp-elf.tar.gz" \
        | sha256sum --check --strict \
    && mkdir -p /opt/espressif \
    && tar -xzf /tmp/riscv32-esp-elf.tar.gz -C /opt/espressif \
    && rm /tmp/riscv32-esp-elf.tar.gz

ENV PATH="/opt/espressif/riscv32-esp-elf/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo Emulator Espressif RISC-V GCC toolchain"
LABEL org.opencontainers.image.source="toolchains/riscv32-esp-gcc/Dockerfile"

WORKDIR /workspace/src
