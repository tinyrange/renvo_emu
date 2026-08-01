FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        binutils-avr=2.26.20160125+Atmel3.7.0-2 \
        gcc-avr=1:7.3.0+Atmel3.7.0-1 \
        avr-libc=1:2.0.0+Atmel3.7.0-1 \
        unzip=6.0-28ubuntu4.1 \
    && rm -rf /var/lib/apt/lists/*

ADD https://packs.download.microchip.com/Microchip.ATmega_DFP.3.6.299.atpack /tmp/atmega.atpack
RUN echo "9be3dea84cf018c52d5a7499d57f8b246d410d701f9703f028250d1d18db8123  /tmp/atmega.atpack" | sha256sum -c - \
    && mkdir -p /opt/microchip/atmega-dfp \
    && unzip -q /tmp/atmega.atpack -d /opt/microchip/atmega-dfp \
    && rm /tmp/atmega.atpack

LABEL org.opencontainers.image.title="Renvo Emulator ATmega328PB AVR GNU toolchain"
LABEL org.opencontainers.image.source="toolchains/avr-gcc/Dockerfile"
LABEL org.opencontainers.image.atmega-dfp="3.6.299"

WORKDIR /workspace/src
