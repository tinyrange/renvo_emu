#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

cargo build -q -p remu-cli
remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/esp32c6-peripherals
rm -rf "$root"
mkdir -p "$root/source/soc" "$root/out"

cp corpus/vendor/esp-idf-c6/start.S "$root/source/start.S"
cp corpus/vendor/esp-idf-c6/link.ld "$root/source/link.ld"
cp corpus/vendor/esp-idf-c6/peripheral_probe.c "$root/source/peripheral_probe.c"
cp corpus/vendor/esp-idf-c6/soc.h "$root/source/soc/soc.h"

idf_commit=7101770dc6db2667b3c477cc31365dd1acd6db4e
while read -r expected header
do
    source_dir=register
    if [ "$header" = efuse_defs.h ]
    then
        source_dir=include
    fi
    url="https://raw.githubusercontent.com/espressif/esp-idf/$idf_commit/components/soc/esp32c6/$source_dir/soc/$header"
    output="$root/source/soc/$header"
    curl --fail --location --silent --show-error "$url" --output "$output"
    actual=$(sha256sum "$output" | cut -d ' ' -f 1)
    if [ "$actual" != "$expected" ]
    then
        echo "ESP-IDF header hash mismatch for $header: expected $expected, got $actual" >&2
        exit 1
    fi
done < corpus/vendor/esp-idf-c6/headers.sha256

mkdir -p "$root/radio-source/soc" "$root/radio-source/modem" "$root/radio-out"
cp corpus/vendor/esp-idf-c6/start.S "$root/radio-source/start.S"
cp corpus/vendor/esp-idf-c6/link.ld "$root/radio-source/link.ld"
cp corpus/vendor/esp-idf-c6/radio_probe.c "$root/radio-source/radio_probe.c"
cp corpus/vendor/esp-idf-c6/soc.h "$root/radio-source/soc/soc.h"
cp "$root/source/soc/reg_base.h" "$root/radio-source/soc/reg_base.h"

fetch_radio_header()
{
    url=$1
    expected=$2
    output=$3
    curl --fail --location --silent --show-error "$url" --output "$output"
    actual=$(sha256sum "$output" | cut -d ' ' -f 1)
    if [ "$actual" != "$expected" ]
    then
        echo "radio header hash mismatch for $url: expected $expected, got $actual" >&2
        exit 1
    fi
}

fetch_radio_header \
    "https://raw.githubusercontent.com/espressif/esp-idf/$idf_commit/components/soc/esp32c6/register/soc/ieee802154_reg.h" \
    2d19c130356367a268b74652a116fc687092900ac05e5665ca5c7cf99596883e \
    "$root/radio-source/soc/ieee802154_reg.h"
fetch_radio_header \
    "https://raw.githubusercontent.com/espressif/esp-idf/$idf_commit/components/soc/esp32c6/include/modem/reg_base.h" \
    83bf8450e9825cedbf901a0be63dad03be9ff27c49663c3b02e724f1c6c517ef \
    "$root/radio-source/modem/reg_base.h"
fetch_radio_header \
    "https://raw.githubusercontent.com/espressif/esp-idf/$idf_commit/components/soc/esp32c6/include/modem/modem_syscon_reg.h" \
    7e08b6db172a81c665c70f0b3dafb85df04a5661c17b1acb0af6df67bdf719e7 \
    "$root/radio-source/modem/modem_syscon_reg.h"
fetch_radio_header \
    "https://raw.githubusercontent.com/espressif/esp-idf/$idf_commit/components/soc/esp32c6/include/modem/modem_lpcon_reg.h" \
    22153f97481d511f747cf99a38fee5178c9a5b92a9dd6e5b1a0f61f38ef7b86b \
    "$root/radio-source/modem/modem_lpcon_reg.h"

"$remu" corpus build \
    --toolchain toolchains/riscv32-esp-gcc-esp32c6.toml \
    --source "$root/radio-source" \
    --output "$root/radio-out" \
    --target esp32c6 \
    --artifact "$root/radio-build.json" \
    -- -O2 -I. start.S radio_probe.c -Wl,-T,link.ld -o /workspace/out/radio-probe.elf

"$remu" run \
    --target esp32c6 \
    --elf "$root/radio-out/radio-probe.elf" \
    --max-instructions 20000 \
    --bus-log "$root/radio-bus.json" \
    --result "$root/radio-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/radio-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32c6.ieee802154") and any(.[]; .region == "esp32c6.modem-syscon")' "$root/radio-bus.json" >/dev/null

"$remu" corpus build \
    --toolchain toolchains/riscv32-esp-gcc-esp32c6.toml \
    --source "$root/source" \
    --output "$root/out" \
    --target esp32c6 \
    --artifact "$root/build.json" \
    -- -O2 -I. start.S peripheral_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32c6 \
    --elf "$root/out/probe.elf" \
    --max-instructions 20000 \
    --bus-log "$root/bus.json" \
    --result "$root/run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/run.json" >/dev/null

for region in \
    esp32c6.i2c0 esp32c6.spi2 esp32c6.i2s esp32c6.ledc esp32c6.rmt \
    esp32c6.pcnt esp32c6.mcpwm esp32c6.parlio esp32c6.gdma esp32c6.saradc \
    esp32c6.etm esp32c6.systimer esp32c6.lp-uart esp32c6.lp-i2c \
    esp32c6.lp-watchdog esp32c6.aes esp32c6.sha esp32c6.hmac esp32c6.rsa \
    esp32c6.digital-signature esp32c6.ecc esp32c6.efuse esp32c6.io-mux \
    esp32c6.interrupt-matrix esp32c6.interrupt-priority esp32c6.uhci0 \
    esp32c6.twai0 esp32c6.hinf esp32c6.timer-group0 \
    esp32c6.plic-machine esp32c6.plic-user esp32c6.clint esp32c6.extmem \
    esp32c6.pmu esp32c6.lp-aon esp32c6.lp-timer
do
    jq -e --arg region "$region" 'any(.[]; .region == $region)' "$root/bus.json" >/dev/null
done
