FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        build-essential=12.10ubuntu1 \
        bzip2=1.0.8-5.1build0.1 \
        ca-certificates=20260601~24.04.1 \
        cmake=3.28.3-1build7 \
        curl=8.5.0-2ubuntu10.11 \
        git=1:2.43.0-1ubuntu7.3 \
        make=4.3-4.1build2 \
        python3=3.12.3-0ubuntu2.1 \
    && rm -rf /var/lib/apt/lists/*

ARG ARM_GCC_URL=https://developer.arm.com/-/media/Files/downloads/gnu-rm/9-2020q2/gcc-arm-none-eabi-9-2020-q2-update-x86_64-linux.tar.bz2
ARG ARM_GCC_SHA256=5adc2ee03904571c2de79d5cfc0f7fe2a5c5f54f44da5b645c17ee57b217f11f

RUN curl --fail --location --proto '=https' --tlsv1.2 \
        --output /tmp/arm-gcc.tar.bz2 "$ARM_GCC_URL" \
    && echo "$ARM_GCC_SHA256  /tmp/arm-gcc.tar.bz2" | sha256sum --check --strict \
    && mkdir -p /opt/arm-gcc \
    && tar -xjf /tmp/arm-gcc.tar.bz2 --strip-components=1 -C /opt/arm-gcc \
    && rm /tmp/arm-gcc.tar.bz2

LABEL org.opencontainers.image.title="Renvo Emulator RP2040 boot-ROM reference builder"
LABEL org.opencontainers.image.source="toolchains/rp2040-bootrom/Dockerfile"

ENV PATH=/opt/arm-gcc/bin:$PATH

WORKDIR /source
