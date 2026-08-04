#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. scripts/lib/toolchain-images.sh

xtensa_image=$(resolve_toolchain_image \
    sha256:e0c54aeaae63f842234ec88f7b5a61b69bfa4d9005ba7490df47328e0dc9892f \
    remu/xtensa-esp-gcc:local)
docker image inspect "$xtensa_image" >/dev/null

cargo build -q -p remu-cli
remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/esp32s3-peripherals
rm -rf "$root"
mkdir -p "$root/out"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source corpus/smoke/xtensa-peripherals \
    --output "$root/out" \
    --target esp32s3 \
    --artifact "$root/build.json" \
    -- -O2 main.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/bus.json" \
    --result "$root/run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/run.json" >/dev/null

for region in \
    esp32s3.uart1 esp32s3.uart2 \
    esp32s3.i2c0 esp32s3.i2c1 esp32s3.spi2 esp32s3.spi3 \
    esp32s3.i2s0 esp32s3.i2s1 esp32s3.rmt esp32s3.ledc esp32s3.pcnt \
    esp32s3.mcpwm0 esp32s3.mcpwm1 esp32s3.twai esp32s3.gdma \
    esp32s3.saradc esp32s3.tsens esp32s3.lcd-cam esp32s3.sdmmc \
    esp32s3.sha esp32s3.aes esp32s3.efuse esp32s3.hmac esp32s3.rsa \
    esp32s3.digital-signature esp32s3.rtc-control esp32s3.interrupt-matrix
do
    jq -e --arg region "$region" 'any(.[]; .region == $region)' "$root/bus.json" >/dev/null
done

echo "ESP32-S3 vendor-toolchain peripheral qualification passed; artifacts: $root"
