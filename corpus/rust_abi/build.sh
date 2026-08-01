#!/bin/sh
set -eu

target=${1:?target triple is required}
profile=${2:?machine profile is required}
optimization=${3:?optimization level is required}
output=${4:?output filename is required}

case "$optimization" in
    0|2|s) ;;
    *) echo "unsupported Rust optimization level: $optimization" >&2; exit 2 ;;
esac

case "$profile" in
    wch) linker=link-wch.ld ;;
    esp32c6) linker=link-esp32c6.ld ;;
    rp2040|rp2350_arm) linker=link-arm.ld ;;
    rp2350_riscv) linker=link-rp2350-riscv.ld ;;
    *) echo "unsupported Rust machine profile: $profile" >&2; exit 2 ;;
esac

build_dir=/tmp/remu-rust-target
export CARGO_TARGET_DIR=$build_dir
export RUSTFLAGS="-C linker=rust-lld -C link-arg=-T$linker -C link-arg=--gc-sections --cfg remu_$profile"

if [ "$optimization" = s ]; then
    opt_config='profile.release.opt-level="s"'
else
    opt_config="profile.release.opt-level=$optimization"
fi

cargo build --release --target "$target" --offline --config "$opt_config"
cp "$build_dir/$target/release/remu-rust-abi" "/workspace/out/$output"
