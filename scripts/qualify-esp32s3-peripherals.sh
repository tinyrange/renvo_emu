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
    esp32s3.i2c0 esp32s3.i2c1 esp32s3.io-mux esp32s3.uhci0 esp32s3.syscon esp32s3.spi2 esp32s3.spi3 \
    esp32s3.i2s0 esp32s3.i2s1 esp32s3.rmt esp32s3.ledc esp32s3.pcnt \
    esp32s3.mcpwm0 esp32s3.mcpwm1 esp32s3.twai esp32s3.gdma \
    esp32s3.saradc esp32s3.tsens esp32s3.lcd-cam esp32s3.sdmmc \
    esp32s3.sha esp32s3.aes esp32s3.efuse esp32s3.hmac esp32s3.rsa \
    esp32s3.digital-signature esp32s3.rtc-control esp32s3.interrupt-matrix
do
    jq -e --arg region "$region" 'any(.[]; .region == $region)' "$root/bus.json" >/dev/null
done

# Compile and run an adapter against Espressif's byte-exact, pinned IO MUX
# register header. This checks that the functional model accepts the same
# addresses and field encodings used by the vendor SDK rather than only the
# hand-written native probe above.
esp_idf_commit=f992ff36f68a783d786d83178e5f85e9a9c76ead
io_mux_header_hash=be80664117c532e67f516262ac0de4621a53b16d2139a4fea7b8acb0f7b45cf2
vendor_source="$root/vendor-io-mux-source"
mkdir -p "$vendor_source/soc" "$root/vendor-io-mux-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_source/link.ld"
cp corpus/vendor/esp-idf/io_mux_probe.c "$vendor_source/io_mux_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/io_mux_reg.h" \
    "$io_mux_header_hash" "$vendor_source/soc/io_mux_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_source" \
    --output "$root/vendor-io-mux-out" \
    --target esp32s3 \
    --artifact "$root/vendor-io-mux-build.json" \
    -- -O2 -I. io_mux_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-io-mux-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-io-mux-bus.json" \
    --result "$root/vendor-io-mux-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-io-mux-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.io-mux")' "$root/vendor-io-mux-bus.json" >/dev/null

# Exercise UHCI0 through Espressif's pinned register header. ESP32-S3 exposes
# one supported UHCI instance, shared by UART0/1/2 and GDMA trigger 2.
uhci_header_hash=491b084dbb1e72fa235a10b827a5bd1b16451bb73e1eea1d25c0dbb02337b1fc
vendor_uhci_source="$root/vendor-uhci-source"
mkdir -p "$vendor_uhci_source/soc" "$root/vendor-uhci-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_uhci_source/link.ld"
cp corpus/vendor/esp-idf/uhci_probe.c "$vendor_uhci_source/uhci_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_uhci_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/uhci_reg.h" \
    "$uhci_header_hash" "$vendor_uhci_source/soc/uhci_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_uhci_source" \
    --output "$root/vendor-uhci-out" \
    --target esp32s3 \
    --artifact "$root/vendor-uhci-build.json" \
    -- -O2 -I. uhci_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-uhci-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-uhci-bus.json" \
    --result "$root/vendor-uhci-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-uhci-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.uhci0")' "$root/vendor-uhci-bus.json" >/dev/null

# Verify clock/tick resets and sticky external-memory permission locking
# against Espressif's pinned SYSCON register definitions.
syscon_header_hash=7bdc24adb9a4a90cb1b5e755d2de2be811b2fbbbc59bfd70af833e1aa3befaa4
vendor_syscon_source="$root/vendor-syscon-source"
mkdir -p "$vendor_syscon_source/soc" "$root/vendor-syscon-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_syscon_source/link.ld"
cp corpus/vendor/esp-idf/syscon_probe.c "$vendor_syscon_source/syscon_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_syscon_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/syscon_reg.h" \
    "$syscon_header_hash" "$vendor_syscon_source/soc/syscon_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_syscon_source" \
    --output "$root/vendor-syscon-out" \
    --target esp32s3 \
    --artifact "$root/vendor-syscon-build.json" \
    -- -O2 -I. syscon_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-syscon-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-syscon-bus.json" \
    --result "$root/vendor-syscon-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-syscon-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.syscon")' "$root/vendor-syscon-bus.json" >/dev/null

echo "ESP32-S3 vendor-toolchain peripheral qualification passed; artifacts: $root"
