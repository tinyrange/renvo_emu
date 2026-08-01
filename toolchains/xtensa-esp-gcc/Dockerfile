FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ADD https://github.com/espressif/crosstool-NG/releases/download/esp-14.2.0_20260121/xtensa-esp-elf-14.2.0_20260121-x86_64-linux-gnu.tar.gz /tmp/xtensa-esp-elf.tar.gz

RUN echo "4e090a6cbf1ff7769684d9a248c9b8bdbe4c0ada098adda54b4c67a91449afa7  /tmp/xtensa-esp-elf.tar.gz" \
        | sha256sum --check --strict \
    && mkdir -p /opt/espressif \
    && tar -xzf /tmp/xtensa-esp-elf.tar.gz -C /opt/espressif \
    && rm /tmp/xtensa-esp-elf.tar.gz

ENV PATH="/opt/espressif/xtensa-esp-elf/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo Emulator Espressif Xtensa GCC toolchain"
LABEL org.opencontainers.image.source="toolchains/xtensa-esp-gcc/Dockerfile"

WORKDIR /workspace/src
