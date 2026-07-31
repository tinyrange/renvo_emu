FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        binutils-arm-none-eabi=2.42-1ubuntu1+23 \
        binutils-riscv64-unknown-elf=2.42-1ubuntu1+6 \
        clang-18=1:18.1.3-1ubuntu1 \
        gcc-arm-none-eabi=15:13.2.rel1-2 \
        gcc-riscv64-unknown-elf=13.2.0-11ubuntu1+12 \
        lld-18=1:18.1.3-1ubuntu1 \
    && rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.title="Renvo freestanding cross-LLVM toolchains"
LABEL org.opencontainers.image.source="toolchains/cross-llvm/Dockerfile"

WORKDIR /workspace/src
