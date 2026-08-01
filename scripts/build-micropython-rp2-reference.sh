#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir=${MICROPYTHON_SOURCE:-"$repo_root/.remu/reference/micropython-v1.28.0"}
output_dir=${MICROPYTHON_OUTPUT:-"$repo_root/.remu/reference/micropython-build-rpi-pico-gcc13"}
image=${MICROPYTHON_BUILD_IMAGE:-remu/micropython-rp2:local}
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
    --env "REMU_BOARD=$board" \
    --env "REMU_VARIANT=$variant" \
    --env "REMU_PLATFORM=$platform" \
    --env "REMU_GCC_TRIPLE=$gcc_triple" \
    --env "REMU_TOOLCHAIN_PATH=$toolchain_path" \
    "$image" \
    sh -eu -c 'make -C /source/mpy-cross -j2 && env ${REMU_GCC_TRIPLE:+PICO_GCC_TRIPLE=$REMU_GCC_TRIPLE} ${REMU_TOOLCHAIN_PATH:+PICO_TOOLCHAIN_PATH=$REMU_TOOLCHAIN_PATH} make -C /source/ports/rp2 BOARD="$REMU_BOARD" ${REMU_VARIANT:+BOARD_VARIANT=$REMU_VARIANT} BUILD=/output CMAKE_ARGS="-DMICROPY_BOARD=$REMU_BOARD -DMICROPY_BOARD_DIR=/source/ports/rp2/boards/$REMU_BOARD${REMU_VARIANT:+ -DMICROPY_BOARD_VARIANT=$REMU_VARIANT} -DPICO_NO_PICOTOOL=1${REMU_PLATFORM:+ -DPICO_PLATFORM=$REMU_PLATFORM}${REMU_GCC_TRIPLE:+ -DPICO_GCC_TRIPLE=$REMU_GCC_TRIPLE}${REMU_TOOLCHAIN_PATH:+ -DPICO_TOOLCHAIN_PATH=$REMU_TOOLCHAIN_PATH}" -j2'

test -s "$output_dir/firmware.elf"
sha256sum "$output_dir/firmware.elf"
