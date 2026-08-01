FROM ubuntu@sha256:8feb4d8ca5354def3d8fce243717141ce31e2c428701f6682bd2fafe15388214

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        bzip2=1.0.8-2 \
        unzip=6.0-25ubuntu1.2 \
    && rm -rf /var/lib/apt/lists/*

ADD https://dr-download.ti.com/software-development/ide-configuration-compiler-or-debugger/MD-LlCjWuAbzH/9.3.1.2/msp430-gcc-9.3.1.11_linux64.tar.bz2 /tmp/msp430-gcc.tar.bz2
ADD https://dr-download.ti.com/software-development/ide-configuration-compiler-or-debugger/MD-LlCjWuAbzH/9.3.1.2/msp430-gcc-support-files-1.212.zip /tmp/msp430-support.zip

RUN echo "b60851b61565e3ddd2193298a3fac5120c2402597174106cc5f8482b9e9986e6  /tmp/msp430-gcc.tar.bz2" | sha256sum -c - \
    && echo "3b1a39f10a344dfefb767e60ac35becef4c065013be86993195b138a5fb0b8d6  /tmp/msp430-support.zip" | sha256sum -c - \
    && mkdir -p /opt/ti \
    && tar xjf /tmp/msp430-gcc.tar.bz2 -C /opt/ti \
    && unzip -q /tmp/msp430-support.zip -d /opt/ti \
    && mv /opt/ti/msp430-gcc-9.3.1.11_linux64 /opt/ti/msp430-gcc \
    && rm /tmp/msp430-gcc.tar.bz2 /tmp/msp430-support.zip

ENV PATH="/opt/ti/msp430-gcc/bin:${PATH}"

LABEL org.opencontainers.image.title="Renvo TI MSP430 GCC toolchain"
LABEL org.opencontainers.image.source="toolchains/msp430-gcc/Dockerfile"
LABEL org.opencontainers.image.msp430-gcc="9.3.1.11"
LABEL org.opencontainers.image.msp430-support="1.212"

WORKDIR /workspace/src
