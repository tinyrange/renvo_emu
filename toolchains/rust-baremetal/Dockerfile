FROM rust@sha256:5c6f46a6e4472ab1ca7ba7d494e6677f2f219ebc02f32025d3986f057635ec9c

RUN rustup component add rust-src \
    && rustup target add \
        riscv32imac-unknown-none-elf \
        thumbv6m-none-eabi \
        thumbv8m.main-none-eabi

COPY preload /opt/remu-rust-preload
RUN cd /opt/remu-rust-preload \
    && RUSTC_BOOTSTRAP=1 cargo build --release \
        -Zbuild-std=core \
        --target riscv32e-unknown-none-elf \
    && sysroot="$(rustc --print sysroot)" \
    && mkdir -p "$sysroot/lib/rustlib/riscv32e-unknown-none-elf/lib" \
    && cp target/riscv32e-unknown-none-elf/release/deps/lib*.rlib \
        "$sysroot/lib/rustlib/riscv32e-unknown-none-elf/lib/"

LABEL org.opencontainers.image.title="Renvo Emulator freestanding Rust toolchain"
LABEL org.opencontainers.image.source="toolchains/rust-baremetal/Dockerfile"

WORKDIR /workspace/src
