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
    esp32s3.i2c0 esp32s3.i2c1 esp32s3.rtc-i2c esp32s3.rtc-io esp32s3.sdm esp32s3.io-mux esp32s3.uhci0 esp32s3.uhci1 esp32s3.peri-backup esp32s3.assist-debug esp32s3.syscon esp32s3.usb-wrap esp32s3.sensitive esp32s3.world-controller esp32s3.extmem esp32s3.xts-aes esp32s3.spi2 esp32s3.spi3 \
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

# Exercise both UHCI controllers through Espressif's pinned register header.
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
jq -e 'any(.[]; .region == "esp32s3.uhci0") and any(.[]; .region == "esp32s3.uhci1")' "$root/vendor-uhci-bus.json" >/dev/null

# Exercise APB-to-retention-memory backup through Espressif's pinned register
# definitions, including functional completion and interrupt clearing.
peri_backup_header_hash=f658cdd19e0392723a571328b91b0b920eba6b52d8cfef30d7709ca606ba49fa
vendor_peri_backup_source="$root/vendor-peri-backup-source"
mkdir -p "$vendor_peri_backup_source/soc" "$root/vendor-peri-backup-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_peri_backup_source/link.ld"
cp corpus/vendor/esp-idf/peri_backup_probe.c "$vendor_peri_backup_source/peri_backup_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_peri_backup_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/peri_backup_reg.h" \
    "$peri_backup_header_hash" "$vendor_peri_backup_source/soc/peri_backup_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_peri_backup_source" \
    --output "$root/vendor-peri-backup-out" \
    --target esp32s3 \
    --artifact "$root/vendor-peri-backup-build.json" \
    -- -O2 -I. peri_backup_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-peri-backup-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-peri-backup-bus.json" \
    --result "$root/vendor-peri-backup-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-peri-backup-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.peri-backup")' "$root/vendor-peri-backup-bus.json" >/dev/null

# Qualify both CPU monitor banks and the functional trace ring against the
# pinned 87-register ASSIST_DEBUG header.
assist_debug_header_hash=8c00fb64855001130a59e5ef67afd5d99ceac26c2d451cc7bd0e176cd24519a2
vendor_assist_debug_source="$root/vendor-assist-debug-source"
mkdir -p "$vendor_assist_debug_source/soc" "$root/vendor-assist-debug-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_assist_debug_source/link.ld"
cp corpus/vendor/esp-idf/assist_debug_probe.c "$vendor_assist_debug_source/assist_debug_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_assist_debug_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/assist_debug_reg.h" \
    "$assist_debug_header_hash" "$vendor_assist_debug_source/soc/assist_debug_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_assist_debug_source" \
    --output "$root/vendor-assist-debug-out" \
    --target esp32s3 \
    --artifact "$root/vendor-assist-debug-build.json" \
    -- -O2 -I. assist_debug_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-assist-debug-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-assist-debug-bus.json" \
    --result "$root/vendor-assist-debug-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-assist-debug-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.assist-debug")' "$root/vendor-assist-debug-bus.json" >/dev/null

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

# Compile USB PHY/pad control against Espressif's pinned wrapper header and
# exercise internal-PHY reset state, software pull control, and pad test mode.
usb_wrap_header_hash=55eb07e728745a919d2025d61a55b95fd9919353fd89c4e02395b44b1d64a1c7
vendor_usb_wrap_source="$root/vendor-usb-wrap-source"
mkdir -p "$vendor_usb_wrap_source/soc" "$root/vendor-usb-wrap-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_usb_wrap_source/link.ld"
cp corpus/vendor/esp-idf/usb_wrap_probe.c "$vendor_usb_wrap_source/usb_wrap_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_usb_wrap_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/usb_wrap_reg.h" \
    "$usb_wrap_header_hash" "$vendor_usb_wrap_source/soc/usb_wrap_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_usb_wrap_source" \
    --output "$root/vendor-usb-wrap-out" \
    --target esp32s3 \
    --artifact "$root/vendor-usb-wrap-build.json" \
    -- -O2 -I. usb_wrap_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-usb-wrap-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-usb-wrap-bus.json" \
    --result "$root/vendor-usb-wrap-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-usb-wrap-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.usb-wrap")' "$root/vendor-usb-wrap-bus.json" >/dev/null

# Compile the permission-control probe against Espressif's pinned sensitive
# register header. It checks vendor reset values, EDMA boundaries, write masks,
# and the sticky lock protecting core-0 peripheral permissions.
sensitive_header_hash=f989baceaf409133537146ca7c377e3502ff792573b40aca48e516b503ba575c
vendor_pms_source="$root/vendor-pms-source"
mkdir -p "$vendor_pms_source/soc" "$root/vendor-pms-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_pms_source/link.ld"
cp corpus/vendor/esp-idf/pms_probe.c "$vendor_pms_source/pms_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_pms_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/sensitive_reg.h" \
    "$sensitive_header_hash" "$vendor_pms_source/soc/sensitive_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_pms_source" \
    --output "$root/vendor-pms-out" \
    --target esp32s3 \
    --artifact "$root/vendor-pms-build.json" \
    -- -O2 -I. pms_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-pms-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-pms-bus.json" \
    --result "$root/vendor-pms-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-pms-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.sensitive")' "$root/vendor-pms-bus.json" >/dev/null

# Exercise both CPU world transitions, write-buffer clearing, switch logging,
# and address-terminated NMI masking through Espressif's pinned WCL header.
world_controller_header_hash=34e5687532c6028c7fe71d04899b49979869635b135117ef012ef733cdb40be2
vendor_wcl_source="$root/vendor-wcl-source"
mkdir -p "$vendor_wcl_source/soc" "$root/vendor-wcl-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_wcl_source/link.ld"
cp corpus/vendor/esp-idf/world_controller_probe.c "$vendor_wcl_source/world_controller_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_wcl_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/world_controller_reg.h" \
    "$world_controller_header_hash" "$vendor_wcl_source/soc/world_controller_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_wcl_source" \
    --output "$root/vendor-wcl-out" \
    --target esp32s3 \
    --artifact "$root/vendor-wcl-build.json" \
    -- -O2 -I. world_controller_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-wcl-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-wcl-bus.json" \
    --result "$root/vendor-wcl-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-wcl-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.world-controller")' "$root/vendor-wcl-bus.json" >/dev/null

# Exercise cache configuration and immediate functional completion through
# Espressif's pinned external-memory/cache register definitions.
extmem_header_hash=1867236e60642887d3b96e8c364e25f404c9750b4b5ac87508a663ac3edf512b
vendor_extmem_source="$root/vendor-extmem-source"
mkdir -p "$vendor_extmem_source/soc" "$root/vendor-extmem-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_extmem_source/link.ld"
cp corpus/vendor/esp-idf/extmem_probe.c "$vendor_extmem_source/extmem_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_extmem_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/extmem_reg.h" \
    "$extmem_header_hash" "$vendor_extmem_source/soc/extmem_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_extmem_source" \
    --output "$root/vendor-extmem-out" \
    --target esp32s3 \
    --artifact "$root/vendor-extmem-build.json" \
    -- -O2 -I. extmem_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-extmem-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-extmem-bus.json" \
    --result "$root/vendor-extmem-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-extmem-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.extmem")' "$root/vendor-extmem-bus.json" >/dev/null

# Exercise RTC-domain I2C reset, clocks, transfer completion, and interrupt
# hierarchy through Espressif's pinned generated register definitions.
rtc_i2c_header_hash=66076f2be4bf7b694a163aac6420d5c7134403bfb89660a6ad1b67100365f6aa
vendor_rtc_i2c_source="$root/vendor-rtc-i2c-source"
mkdir -p "$vendor_rtc_i2c_source/soc" "$root/vendor-rtc-i2c-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_rtc_i2c_source/link.ld"
cp corpus/vendor/esp-idf/rtc_i2c_probe.c "$vendor_rtc_i2c_source/rtc_i2c_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_rtc_i2c_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/rtc_i2c_reg.h" \
    "$rtc_i2c_header_hash" "$vendor_rtc_i2c_source/soc/rtc_i2c_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_rtc_i2c_source" \
    --output "$root/vendor-rtc-i2c-out" \
    --target esp32s3 \
    --artifact "$root/vendor-rtc-i2c-build.json" \
    -- -O2 -I. rtc_i2c_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-rtc-i2c-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-rtc-i2c-bus.json" \
    --result "$root/vendor-rtc-i2c-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-rtc-i2c-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.rtc-i2c")' "$root/vendor-rtc-i2c-bus.json" >/dev/null

# Compile the complete SENS surface against Espressif's generated header and
# prove touch scanning plus the ULP-facing RTC-I2C start path.
sens_header_hash=f3d7fee900f7cdf5d063d1d339e95811f300e35eae6cd96a64d35aacebea2522
vendor_sens_source="$root/vendor-sens-source"
mkdir -p "$vendor_sens_source/soc" "$root/vendor-sens-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_sens_source/link.ld"
cp corpus/vendor/esp-idf/sens_probe.c "$vendor_sens_source/sens_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_sens_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/sens_reg.h" \
    "$sens_header_hash" "$vendor_sens_source/soc/sens_reg.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/rtc_i2c_reg.h" \
    "$rtc_i2c_header_hash" "$vendor_sens_source/soc/rtc_i2c_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_sens_source" \
    --output "$root/vendor-sens-out" \
    --target esp32s3 \
    --artifact "$root/vendor-sens-build.json" \
    -- -O2 -I. sens_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-sens-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-sens-bus.json" \
    --result "$root/vendor-sens-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-sens-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.tsens") and any(.[]; .region == "esp32s3.rtc-i2c")' "$root/vendor-sens-bus.json" >/dev/null

# ESP-IDF exposes the XTS base through its SoC map and controls it through the
# SYSTEM header; the XTS register offsets themselves come from TRM v1.8.
system_header_hash=61d7ab616c6339bfe2e3a704e24ea28ff6305641f064ddf6ceec17cc4ad0c9fd
vendor_xts_source="$root/vendor-xts-source"
mkdir -p "$vendor_xts_source/soc" "$root/vendor-xts-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_xts_source/link.ld"
cp corpus/vendor/esp-idf/xts_aes_probe.c "$vendor_xts_source/xts_aes_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_xts_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/system_reg.h" \
    "$system_header_hash" "$vendor_xts_source/soc/system_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_xts_source" \
    --output "$root/vendor-xts-out" \
    --target esp32s3 \
    --artifact "$root/vendor-xts-build.json" \
    -- -O2 -I. xts_aes_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-xts-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-xts-bus.json" \
    --result "$root/vendor-xts-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-xts-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.xts-aes") and any(.[]; .region == "esp32s3.system")' "$root/vendor-xts-bus.json" >/dev/null

# Compile RTC_IO and GPIO_SD access through the exact pinned Espressif headers.
rtc_io_header_hash=ce037481615f49eb5c4bbf13ab01bd49a30050ebd934b2b1e9804d7e32a1f048
gpio_sd_header_hash=fbfcd8eecc83b5cd60a05d3c8e6597e2e09820c4cf907294624af077206bdde0
vendor_rtc_io_source="$root/vendor-rtc-io-source"
mkdir -p "$vendor_rtc_io_source/soc" "$root/vendor-rtc-io-out"
cp corpus/smoke/xtensa-peripherals/link.ld "$vendor_rtc_io_source/link.ld"
cp corpus/vendor/esp-idf/rtc_io_sdm_probe.c "$vendor_rtc_io_source/rtc_io_sdm_probe.c"
cp corpus/vendor/esp-idf/soc/soc.h "$vendor_rtc_io_source/soc/soc.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/rtc_io_reg.h" \
    "$rtc_io_header_hash" "$vendor_rtc_io_source/soc/rtc_io_reg.h"
fetch_exact \
    "https://raw.githubusercontent.com/espressif/esp-idf/$esp_idf_commit/components/soc/esp32s3/register/soc/gpio_sd_reg.h" \
    "$gpio_sd_header_hash" "$vendor_rtc_io_source/soc/gpio_sd_reg.h"

"$remu" corpus build \
    --toolchain toolchains/xtensa-esp-gcc-esp32s3.toml \
    --source "$vendor_rtc_io_source" \
    --output "$root/vendor-rtc-io-out" \
    --target esp32s3 \
    --artifact "$root/vendor-rtc-io-build.json" \
    -- -O2 -I. rtc_io_sdm_probe.c -Wl,-T,link.ld -o /workspace/out/probe.elf

"$remu" run \
    --target esp32s3 \
    --elf "$root/vendor-rtc-io-out/probe.elf" \
    --max-instructions 10000 \
    --bus-log "$root/vendor-rtc-io-bus.json" \
    --result "$root/vendor-rtc-io-run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/vendor-rtc-io-run.json" >/dev/null
jq -e 'any(.[]; .region == "esp32s3.rtc-io") and any(.[]; .region == "esp32s3.sdm")' "$root/vendor-rtc-io-bus.json" >/dev/null

echo "ESP32-S3 vendor-toolchain peripheral qualification passed; artifacts: $root"
