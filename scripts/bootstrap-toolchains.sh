#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

include_xc8=0
if [ "${1:-}" = "--include-xc8" ]
then
    include_xc8=1
elif [ "$#" -ne 0 ]
then
    echo "usage: $0 [--include-xc8]" >&2
    exit 2
fi

build()
{
    tag=$1
    context=$2
    shift 2
    docker build --pull=false --tag "$tag" "$@" "$context"
    image_id=$(docker image inspect --format '{{.Id}}' "$tag")
    printf '%s  %s\n' "$image_id" "$tag"
}

build remu/cross-gcc:local toolchains/cross-gcc
build remu/cross-llvm:local toolchains/cross-llvm
build remu/xtensa-esp-gcc:local toolchains/xtensa-esp-gcc
build remu/avr-gcc:local toolchains/avr-gcc
build remu/msp430-gcc:local toolchains/msp430-gcc
build remu/sdcc-mcs51:4.5.0 toolchains/sdcc-mcs51
build remu/rust-baremetal:local toolchains/rust-baremetal
build remu/nanoc6-esptool:5.3.0 hardware/nanoc6 \
    --file hardware/nanoc6/Dockerfile.esptool

if [ "$include_xc8" -eq 1 ]
then
    echo "Building XC8 confirms that you have reviewed and accepted its vendor terms." >&2
    build remu-xc8:4.00-pic16f1xxxx-1.31.465 toolchains/xc8
else
    echo "XC8 was not built. Re-run with --include-xc8 after reviewing its vendor terms." >&2
fi

echo "Local toolchain bootstrap complete. Corpus builds resolve these tags to immutable IDs."
