FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive
ARG MQUICKJS_COMMIT=203d5bb79789bc47b74855d9207415dab71661a0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        binutils-arm-none-eabi=2.42-1ubuntu1+23 \
        binutils-riscv64-unknown-elf=2.42-1ubuntu1+6 \
        build-essential=12.10ubuntu1 \
        ca-certificates=20240203 \
        gcc-arm-none-eabi=15:13.2.rel1-2 \
        gcc-riscv64-unknown-elf=13.2.0-11ubuntu1+12 \
        libnewlib-dev=4.4.0.20231231-2 \
    && rm -rf /var/lib/apt/lists/*

ADD https://github.com/espressif/crosstool-NG/releases/download/esp-14.2.0_20260121/xtensa-esp-elf-14.2.0_20260121-x86_64-linux-gnu.tar.gz /tmp/xtensa-esp-elf.tar.gz
ADD https://github.com/bellard/mquickjs/archive/203d5bb79789bc47b74855d9207415dab71661a0.tar.gz /tmp/mquickjs.tar.gz

RUN echo "4e090a6cbf1ff7769684d9a248c9b8bdbe4c0ada098adda54b4c67a91449afa7  /tmp/xtensa-esp-elf.tar.gz" \
        | sha256sum --check --strict \
    && echo "6c241e19ca74ad21c811a819469521c895bfd1c25c56481eb2bb5b3c5004851f  /tmp/mquickjs.tar.gz" \
        | sha256sum --check --strict \
    && mkdir -p /opt/espressif /opt/mquickjs \
    && tar -xzf /tmp/xtensa-esp-elf.tar.gz -C /opt/espressif \
    && tar -xzf /tmp/mquickjs.tar.gz -C /opt/mquickjs --strip-components=1 \
    && make -C /opt/mquickjs mqjs \
    && cp /opt/mquickjs/mqjs /usr/local/bin/mqjs \
    && make -C /opt/mquickjs clean \
    && make -C /opt/mquickjs MQJS_BUILD_FLAGS=-m32 mqjs_stdlib.h mquickjs_atom.h \
    && rm /tmp/xtensa-esp-elf.tar.gz /tmp/mquickjs.tar.gz

ENV PATH="/opt/espressif/xtensa-esp-elf/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo Emulator pinned MQuickJS cross-runtime toolchains"
LABEL org.opencontainers.image.source="toolchains/mquickjs/Dockerfile"
LABEL org.opencontainers.image.revision="${MQUICKJS_COMMIT}"

WORKDIR /workspace
