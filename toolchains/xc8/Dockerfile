FROM ubuntu@sha256:8feb4d8ca5354def3d8fce243717141ce31e2c428701f6682bd2fafe15388214

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        unzip=6.0-25ubuntu1.2 \
        xz-utils=5.2.4-1ubuntu1.1 \
    && rm -rf /var/lib/apt/lists/*

ADD https://ww1.microchip.com/downloads/aemDocuments/documents/DEV/ProductDocuments/SoftwareTools/xc8-v4.00-full-install-linux-x64.tar.xz /tmp/xc8.tar.xz
ADD https://packs.download.microchip.com/Microchip.PIC16F1xxxx_DFP.1.31.465.atpack /tmp/pic16f1xxxx.atpack

RUN echo "a24104a8f1fa3dcd2e18d8e42c08485a6c26d5789960ed8bcc7e6bdc2c6b46c6  /tmp/xc8.tar.xz" | sha256sum -c - \
    && echo "4ab50366f9ab7609c14c1b93e65cd9222d15d62d29fb2252e3ea0b9255dc125f  /tmp/pic16f1xxxx.atpack" | sha256sum -c - \
    && mkdir -p /opt/microchip /opt/microchip/pic16f1xxxx-dfp \
    && tar xJf /tmp/xc8.tar.xz -C /opt/microchip \
    && mv /opt/microchip/xc8-v4.00 /opt/microchip/xc8 \
    && unzip -q /tmp/pic16f1xxxx.atpack -d /opt/microchip/pic16f1xxxx-dfp \
    && rm /tmp/xc8.tar.xz /tmp/pic16f1xxxx.atpack

ENV PATH="/opt/microchip/xc8/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo MPLAB XC8 PIC toolchain"
LABEL org.opencontainers.image.source="toolchains/xc8/Dockerfile"
LABEL org.opencontainers.image.xc8="4.00"
LABEL org.opencontainers.image.device-pack="Microchip.PIC16F1xxxx_DFP.1.31.465"

WORKDIR /workspace/src
