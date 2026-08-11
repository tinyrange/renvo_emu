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
root=.remu/qualification/esp32s3-usb-stacks
rm -rf "$root"
mkdir -p "$root/source/upstream" "$root/out"

fetch_exact()
{
    url=$1
    expected=$2
    output=$3
    curl --fail --location --silent --show-error "$url" --output "$output"
    actual=$(sha256sum "$output" | cut -d ' ' -f 1)
    if [ "$actual" != "$expected" ]
    then
        echo "upstream source hash mismatch for $url: expected $expected, got $actual" >&2
        exit 1
    fi
}

# These revisions are the source commits recorded by the ESP Component Registry
# packages esp_tinyusb 2.2.1 and tinyusb 0.21.0~1 used by ESP-IDF 6.0.2.
esp_usb_commit=8e779566ef71d43928cbf7e125e8eb54bab3f542
esp_usb_archive_hash=1cb92ca261342bc00fbd2dc6d7ecbfa14c81110c217fff11435e39baca9846b4
tinyusb_commit=7049c58a0e895acc92c6407574b05b5536eddfc8
tinyusb_archive_hash=4948db082a46739bc3fab4fdda1bf03139393999213143f7a342e28fa624cb38

fetch_exact \
    "https://codeload.github.com/espressif/esp-usb/tar.gz/$esp_usb_commit" \
    "$esp_usb_archive_hash" "$root/esp-usb.tar.gz"
fetch_exact \
    "https://codeload.github.com/espressif/tinyusb/tar.gz/$tinyusb_commit" \
    "$tinyusb_archive_hash" "$root/tinyusb.tar.gz"

tar -xzf "$root/esp-usb.tar.gz" -C "$root/source/upstream" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/tinyusb.c" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/descriptors_control.c" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/usb_descriptors.c" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/include/tinyusb.h" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/include_private/descriptors_control.h" \
    "esp-usb-$esp_usb_commit/device/esp_tinyusb/include_private/usb_descriptors.h"
tar -xzf "$root/tinyusb.tar.gz" -C "$root/source/upstream" \
    "tinyusb-$tinyusb_commit/src"
cp -R corpus/vendor/esp-idf-tinyusb/. "$root/source/"
cp corpus/vendor/esp-idf/xtensa-start.S "$root/source/start.S"
cp corpus/vendor/esp-idf/xtensa-link.ld "$root/source/link.ld"

esp_tinyusb="upstream/esp-usb-$esp_usb_commit/device/esp_tinyusb"
tinyusb="upstream/tinyusb-$tinyusb_commit"

# Compile the real Espressif install layer and TinyUSB device/core, CDC, FIFO,
# and Synopsys DWC2 sources. The adapter supplies only the RTOS/PHY/interrupt
# shell needed to make the stack a deterministic standalone ELF.
"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$root/source" \
    --output "$root/out" \
    --target esp32s3 \
    --artifact "$root/build.json" \
    -- \
    -O2 -Iinclude -I"$esp_tinyusb/include" -I"$esp_tinyusb/include_private" -I"$tinyusb/src" \
    main.c start.S \
    "$esp_tinyusb/tinyusb.c" \
    "$esp_tinyusb/descriptors_control.c" \
    "$esp_tinyusb/usb_descriptors.c" \
    "$tinyusb/src/tusb.c" \
    "$tinyusb/src/common/tusb_fifo.c" \
    "$tinyusb/src/device/usbd.c" \
    "$tinyusb/src/class/cdc/cdc_device.c" \
    "$tinyusb/src/portable/synopsys/dwc2/dwc2_common.c" \
    "$tinyusb/src/portable/synopsys/dwc2/dcd_dwc2.c" \
    -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/out/probe.elf" \
    --max-instructions 700000 \
    --result "$root/run.json"

# Enumeration includes the device/configuration descriptor requests, address
# and configuration changes, and CDC control-line setup. PASS is then sent over
# the resulting CDC IN endpoint and captured by the emulator host.
jq -e \
    '.reason == "Halted" and .exit_code == 0 and .usb == [80, 65, 83, 83, 10]' \
    "$root/run.json" >/dev/null
