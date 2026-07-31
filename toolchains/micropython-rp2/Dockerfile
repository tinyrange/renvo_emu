FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential=12.10ubuntu1 \
        ca-certificates=20240203 \
        cmake=3.28.3-1build7 \
        gcc-arm-none-eabi=15:13.2.rel1-2 \
        gcc-riscv64-unknown-elf=13.2.0-11ubuntu1+12 \
        git=1:2.43.0-1ubuntu7.3 \
        binutils-riscv64-unknown-elf=2.42-1ubuntu1+6 \
        libnewlib-arm-none-eabi=4.4.0.20231231-2 \
        libstdc++-arm-none-eabi-newlib=15:13.2.rel1-2+26 \
        make=4.3-4.1build2 \
        python3=3.12.3-0ubuntu2.1 \
    && rm -rf /var/lib/apt/lists/*

ARG RPI_RISCV_TOOLCHAIN_URL=https://github.com/raspberrypi/pico-sdk-tools/releases/download/v2.3.0-0/riscv-toolchain-15-x86_64-lin.tar.gz
ARG RPI_RISCV_TOOLCHAIN_SHA256=8f64b0910afbe6425e2cede18599cf290ad1294cde23d08f8008d815b82197fb
ADD ${RPI_RISCV_TOOLCHAIN_URL} /tmp/riscv-toolchain.tar.gz
RUN echo "${RPI_RISCV_TOOLCHAIN_SHA256}  /tmp/riscv-toolchain.tar.gz" | sha256sum --check - \
    && mkdir -p /opt/riscv \
    && tar -xzf /tmp/riscv-toolchain.tar.gz -C /opt/riscv \
    && rm /tmp/riscv-toolchain.tar.gz

LABEL org.opencontainers.image.title="Renvo MicroPython RP2 reference builder"
LABEL org.opencontainers.image.source="toolchains/micropython-rp2/Dockerfile"

WORKDIR /source
