#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

optimization=${1:--O2}
compiler=${2:-gcc}
case "$optimization" in
    -O0|-O2|-Os) ;;
    *)
        echo "usage: $0 [-O0|-O2|-Os] [gcc|clang]" >&2
        exit 2
        ;;
esac
case "$compiler" in
    gcc|clang) ;;
    *)
        echo "usage: $0 [-O0|-O2|-Os] [gcc|clang]" >&2
        exit 2
        ;;
esac

case "$optimization" in
    -O0) variant=o0 ;;
    -O2) variant=o2 ;;
    -Os) variant=os ;;
esac

cargo run -q -p remu-casegen -- corpus/edge_cases
cargo build -q -p remu-cli

remu=target/debug/remu
artifact_root=".remu/edge-$compiler-$variant"
mkdir -p \
    "$artifact_root/wch" \
    "$artifact_root/rp" \
    "$artifact_root/esp" \
    "$artifact_root/arm" \
    "$artifact_root/xtensa" \
    "$artifact_root/artifacts"

build_batch()
{
    name=$1
    toolchain=$2
    target=$3
    shift 3
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/edge_cases \
        --output "$artifact_root/$name" \
        --target "$target" \
        --artifact "$artifact_root/artifacts/$name.json" \
        --timeout-seconds 900 \
        -- "$@" "$optimization"
}

run_batch()
{
    target=$1
    name=$2
    "$remu" corpus run \
        --target "$target" \
        --input "$artifact_root/$name" \
        --manifest corpus/edge_cases/manifest.tsv \
        --max-instructions 300000 \
        --artifact "$artifact_root/$target-$name.json"
}

build_batch wch "toolchains/batch-riscv-$compiler.toml" ch32v003 wch
build_batch rp "toolchains/batch-riscv-$compiler.toml" rp2350 rp2350
build_batch esp "toolchains/batch-riscv-$compiler.toml" esp32c6 esp32c6
build_batch arm "toolchains/batch-arm-$compiler.toml" rp2040 cortex-m0plus
if [ "$compiler" = gcc ]
then
    build_batch xtensa toolchains/batch-xtensa-gcc.toml esp32s3
fi

run_batch ch32v003 wch
run_batch ch32v006 wch
run_batch rp2350 rp
run_batch esp32c6 esp
run_batch rp2040 arm
run_batch rp2350 arm
if [ "$compiler" = gcc ]
then
    run_batch esp32s3 xtensa
fi

echo "edge corpus $compiler $optimization passed; artifacts: $artifact_root"
