#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

remu=${REMU_BIN:-target/release/remu}
artifact_root=${RP2040_THROUGHPUT_ARTIFACT_ROOT:-.remu/qualification/rp2040-throughput}
instructions=${RP2040_THROUGHPUT_INSTRUCTIONS:-1000000}
minimum_ips=${RP2040_THROUGHPUT_MINIMUM_IPS:-250000}
cross_image=$(resolve_toolchain_image \
    sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 \
    remu/cross-gcc:local)

if [ ! -x "$remu" ]
then
    cargo build --release --package remu-cli --locked
fi

mkdir -p "$artifact_root/image"
"$remu" corpus build \
    --toolchain toolchains/arm-gcc-cortex-m0plus.toml \
    --source corpus/smoke/rp-throughput \
    --output "$artifact_root/image" \
    --target rp2040 \
    --artifact "$artifact_root/build.json" \
    -- start.S -Wl,-T,link.ld -o /workspace/out/throughput.elf

python3 scripts/benchmark-command.py \
    --label rp2040/interpreter-throughput \
    --actions "$instructions" \
    --output "$artifact_root/metrics.json" \
    --artifact "$artifact_root/run.json" \
    -- "$remu" run \
        --target rp2040 \
        --elf "$artifact_root/image/throughput.elf" \
        --max-instructions "$instructions" \
        --result "$artifact_root/run.json"

jq -e \
    --argjson instructions "$instructions" \
    '.reason == "InstructionLimit" and .stats.instructions == $instructions' \
    "$artifact_root/run.json" >/dev/null
observed_ips=$(jq -er '.actions_per_second' "$artifact_root/metrics.json")
if ! awk -v observed="$observed_ips" -v minimum="$minimum_ips" \
    'BEGIN { exit !(observed >= minimum) }'
then
    echo "RP2040 throughput regression: ${observed_ips} IPS is below ${minimum_ips} IPS" >&2
    exit 1
fi

printf 'RP2040 throughput floor passed: %.0f IPS (minimum %s IPS, image %s)\n' \
    "$observed_ips" "$minimum_ips" "$cross_image"
