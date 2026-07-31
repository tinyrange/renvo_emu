#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir=${MICROPYTHON_SOURCE:-"$repo_root/.renvo/reference/micropython-v1.28.0"}
output_dir=${MICROPYTHON_OUTPUT:-"$repo_root/.renvo/reference/micropython-build-rpi-pico-gcc13"}
image=${MICROPYTHON_BUILD_IMAGE:-renvo/micropython-rp2:local}
board=${MICROPYTHON_BOARD:-RPI_PICO}
variant=${MICROPYTHON_BOARD_VARIANT:-}
platform=${MICROPYTHON_PLATFORM:-}
gcc_triple=${MICROPYTHON_GCC_TRIPLE:-}
toolchain_path=${MICROPYTHON_TOOLCHAIN_PATH:-}

test -f "$source_dir/ports/rp2/Makefile"
mkdir -p "$output_dir"

docker build --tag "$image" "$repo_root/toolchains/micropython-rp2"
docker run --rm \
    --network none \
    --mount "type=bind,src=$source_dir,dst=/source" \
    --mount "type=bind,src=$output_dir,dst=/output" \
    --env "RENVO_BOARD=$board" \
    --env "RENVO_VARIANT=$variant" \
    --env "RENVO_PLATFORM=$platform" \
    --env "RENVO_GCC_TRIPLE=$gcc_triple" \
    --env "RENVO_TOOLCHAIN_PATH=$toolchain_path" \
    "$image" \
    sh -eu -c 'make -C /source/mpy-cross -j2 && env ${RENVO_GCC_TRIPLE:+PICO_GCC_TRIPLE=$RENVO_GCC_TRIPLE} ${RENVO_TOOLCHAIN_PATH:+PICO_TOOLCHAIN_PATH=$RENVO_TOOLCHAIN_PATH} make -C /source/ports/rp2 BOARD="$RENVO_BOARD" ${RENVO_VARIANT:+BOARD_VARIANT=$RENVO_VARIANT} BUILD=/output CMAKE_ARGS="-DMICROPY_BOARD=$RENVO_BOARD -DMICROPY_BOARD_DIR=/source/ports/rp2/boards/$RENVO_BOARD${RENVO_VARIANT:+ -DMICROPY_BOARD_VARIANT=$RENVO_VARIANT} -DPICO_NO_PICOTOOL=1${RENVO_PLATFORM:+ -DPICO_PLATFORM=$RENVO_PLATFORM}${RENVO_GCC_TRIPLE:+ -DPICO_GCC_TRIPLE=$RENVO_GCC_TRIPLE}${RENVO_TOOLCHAIN_PATH:+ -DPICO_TOOLCHAIN_PATH=$RENVO_TOOLCHAIN_PATH}" -j2'

test -s "$output_dir/firmware.elf"
sha256sum "$output_dir/firmware.elf"
