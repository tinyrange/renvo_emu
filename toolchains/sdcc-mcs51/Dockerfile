FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90 AS build

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        bison \
        build-essential \
        bzip2 \
        flex \
        libboost-dev \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

ADD https://downloads.sourceforge.net/project/sdcc/sdcc/4.5.0/sdcc-src-4.5.0.tar.bz2 /tmp/sdcc.tar.bz2

RUN echo "d5030437fb436bb1d93a8dbdbfb46baaa60613318f4fb3f5871d72815d1eed80  /tmp/sdcc.tar.bz2" | sha256sum -c - \
    && tar xjf /tmp/sdcc.tar.bz2 -C /tmp \
    && cd /tmp/sdcc-4.5.0 \
    && ./configure \
        --prefix=/opt/sdcc \
        --disable-z80-port \
        --disable-z180-port \
        --disable-r2k-port \
        --disable-r2ka-port \
        --disable-r3ka-port \
        --disable-sm83-port \
        --disable-tlcs90-port \
        --disable-ez80_z80-port \
        --disable-z80n-port \
        --disable-r800-port \
        --disable-ds390-port \
        --disable-ds400-port \
        --disable-pic14-port \
        --disable-pic16-port \
        --disable-hc08-port \
        --disable-s08-port \
        --disable-stm8-port \
        --disable-pdk13-port \
        --disable-pdk14-port \
        --disable-pdk15-port \
        --disable-mos6502-port \
        --disable-mos65c02-port \
        --disable-f8-port \
        --disable-ucsim \
        --disable-doc \
    && make -j2 \
    && make install

FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

COPY --from=build /opt/sdcc /opt/sdcc

ENV PATH="/opt/sdcc/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo Emulator source-built SDCC MCS-51 toolchain"
LABEL org.opencontainers.image.source="toolchains/sdcc-mcs51/Dockerfile"
LABEL org.opencontainers.image.sdcc="4.5.0"
LABEL org.opencontainers.image.sdcc-source-sha256="d5030437fb436bb1d93a8dbdbfb46baaa60613318f4fb3f5871d72815d1eed80"

WORKDIR /workspace/src
