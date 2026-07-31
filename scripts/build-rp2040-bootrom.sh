#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir=${RP2040_BOOTROM_SOURCE:-"$repo_root/.renvo/reference/pico-bootrom"}
output_dir=${RP2040_BOOTROM_OUTPUT:-"$repo_root/.renvo/reference/pico-bootrom-build"}
image=${RP2040_BOOTROM_IMAGE:-renvo/rp2040-bootrom:local}

if [ ! -f "$source_dir/CMakeLists.txt" ]; then
    echo "RP2040 boot-ROM source is missing at $source_dir" >&2
    exit 1
fi

mkdir -p "$output_dir"
docker build --tag "$image" "$repo_root/toolchains/rp2040-bootrom"
docker run --rm \
    --network none \
    --read-only \
    --tmpfs /tmp:rw,noexec,nosuid,size=64m \
    --mount "type=bind,src=$source_dir,dst=/source,readonly" \
    --mount "type=bind,src=$output_dir,dst=/output" \
    --env GIT_CONFIG_COUNT=1 \
    --env GIT_CONFIG_KEY_0=safe.directory \
    --env GIT_CONFIG_VALUE_0=/source \
    --env GIT_DIR=/source/.git \
    --env GIT_WORK_TREE=/source \
    "$image" \
    sh -eu -c 'cmake -S /source -B /output -DCMAKE_BUILD_TYPE=Debug && cmake --build /output --parallel'

test -s "$output_dir/bootrom.bin"
sha256sum "$output_dir/bootrom.bin"
