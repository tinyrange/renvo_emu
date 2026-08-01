FROM ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates=20260601~24.04.1 \
        curl=8.5.0-2ubuntu10.11 \
    && rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.title="Renvo Emulator official firmware fetcher"
LABEL org.opencontainers.image.source="toolchains/firmware-fetch/Dockerfile"

ENTRYPOINT ["/usr/bin/curl"]
