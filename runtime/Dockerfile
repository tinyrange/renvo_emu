# Runtime-only image. Compiler/toolchain images live under toolchains/ and are
# intentionally not copied into this image.
FROM ubuntu@sha256:3131b4cc82a783df6c9df078f86e01819a13594b865c2cad47bd1bca2b7063bb

ARG REMU_SOURCE_COMMIT=unknown
ARG REMU_VERSION=unknown

RUN groupadd --gid 10001 remu \
    && useradd --uid 10001 --gid 10001 --create-home --shell /usr/sbin/nologin remu \
    && mkdir --mode=0755 /work \
    && chown 10001:10001 /work

COPY remu /usr/local/bin/remu
RUN chmod 0755 /usr/local/bin/remu

LABEL org.opencontainers.image.title="Renvo Emulator runtime" \
      org.opencontainers.image.description="Deterministic MCU firmware emulator (runtime only)" \
      org.opencontainers.image.source="https://github.com/tinyrange/renvo_emu" \
      org.opencontainers.image.revision="${REMU_SOURCE_COMMIT}" \
      org.opencontainers.image.version="${REMU_VERSION}"

USER 10001:10001
WORKDIR /work
ENTRYPOINT ["/usr/local/bin/remu"]
