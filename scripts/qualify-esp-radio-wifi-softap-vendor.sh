#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

chip=${1:-}
case "$chip" in
    esp32c6|esp32s3) ;;
    *) echo "usage: $0 esp32c6|esp32s3" >&2; exit 2 ;;
esac

requirements=qualification/radio/wifi-softap-vendor-requirements.json
artifact_root=${REMU_RADIO_WIFI_SOFTAP_ROOT:-.remu/qualification/radio-wifi-softap-vendor}
chip_root=$artifact_root/$chip
rom_root=$chip_root/roms
build_root=$chip_root/build
idf_image=$(jq -r '.esp_idf_container' "$requirements")
firmware_project=$(jq -r '.firmware_project' "$requirements")
firmware_elf=$(jq -r '.firmware_elf' "$requirements")
rom_file=$(jq -r --arg chip "$chip" '.chips[$chip].rom_file' "$requirements")
expected_rom_sha=$(jq -r --arg chip "$chip" '.chips[$chip].rom_sha256' "$requirements")
minimum_instructions=$(jq -r --arg chip "$chip" '.chips[$chip].minimum_instructions' "$requirements")
radio_input=$(jq -r --arg chip "$chip" '.chips[$chip].radio_input' "$requirements")
rom_start=$(jq -r --arg chip "$chip" '.chips[$chip].rom_start' "$requirements")
rom_end=$(jq -r --arg chip "$chip" '.chips[$chip].rom_end' "$requirements")
expected_ap_mac=$(jq -c --arg chip "$chip" '.chips[$chip].expected_ap_mac' "$requirements")
expected_beacon_length=$(jq -r --arg chip "$chip" '.chips[$chip].expected_beacon_length' "$requirements")
expected_authentication_response_length=$(jq -r --arg chip "$chip" '.chips[$chip].expected_authentication_response_length' "$requirements")
expected_association_response_length=$(jq -r --arg chip "$chip" '.chips[$chip].expected_association_response_length' "$requirements")

rm -rf "$chip_root"
mkdir -p "$chip_root"
chip_root_absolute=$(CDPATH= cd -- "$chip_root" && pwd)

REMU_ESP_ROM_DIR="$rom_root" scripts/fetch-esp-rom-elfs.sh >"$chip_root/rom-fetch.log"
actual_rom_sha=$(sha256sum "$rom_root/$rom_file" | cut -d ' ' -f 1)
if [ "$actual_rom_sha" != "$expected_rom_sha" ]
then
    echo "$chip ROM hash mismatch: expected $expected_rom_sha, got $actual_rom_sha" >&2
    exit 1
fi

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --env HOME=/tmp \
    --volume "$repo_root:/workspace:ro" \
    --volume "$chip_root_absolute:/out" \
    "$idf_image" \
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/qualification/radio/sdkconfig.rom.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig build && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-wifi-softap-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-wifi-softap-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_softap()
{
    result=$1
    replay=$2
    shift 2
    "$remu" run \
        --target "$chip" \
        --elf "$elf" \
        --boot-rom "$rom_root/$rom_file" \
        --esp-app-image "$flash" \
        --max-instructions "$minimum_instructions" \
        --radio-input "$radio_input" \
        --radio-replay "$replay" \
        --result "$result" \
        "$@"
}

run_vendor_softap "$chip_root/result.json" "$chip_root/radio-replay.json" \
    --coverage "$chip_root/coverage.json"

jq -e --argjson instructions "$minimum_instructions" \
    '.reason == "InstructionLimit" and .stats.instructions == $instructions' \
    "$chip_root/result.json" >/dev/null
jq -e --argjson start "$rom_start_decimal" --argjson end "$rom_end_decimal" \
    'any(.addresses[]; .address >= $start and .address < $end)' \
    "$chip_root/coverage.json" >/dev/null
jq -e --argjson entry "$entry" \
    'any(.addresses[]; .address == $entry)' \
    "$chip_root/coverage.json" >/dev/null

jq -e \
    --argjson ap "$expected_ap_mac" \
    --argjson beacon_length "$expected_beacon_length" \
    --argjson authentication_length "$expected_authentication_response_length" \
    --argjson association_length "$expected_association_response_length" '
    .events as $events |
    ([ $events[] |
        select(.event == "submitted" and
               .request.frame.protocol == "wifi" and
               .request.frame.origin == "host-injection") |
        . as $submitted |
        select(any($events[];
            .event == "reception" and
            .id == $submitted.id and
            .receiver == 1 and
            .outcome.kind == "delivered")) ] | length) == 3 and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        (.request.frame.bytes | length) == 43 and
        .request.frame.bytes[0:2] == [208, 0] and
        .request.frame.bytes[4:10] == [255, 255, 255, 255, 255, 255] and
        .request.frame.bytes[10:16] == $ap and
        .request.frame.bytes[-4:] == [82, 69, 77, 85]) and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        (.request.frame.bytes | length) == $beacon_length and
        .request.frame.bytes[0:2] == [128, 0] and
        .request.frame.bytes[10:16] == $ap) and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        (.request.frame.bytes | length) == $authentication_length and
        .request.frame.bytes[0:2] == [176, 0] and
        .request.frame.bytes[4:10] == [2, 170, 187, 204, 221, 1] and
        .request.frame.bytes[10:16] == $ap) and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        (.request.frame.bytes | length) == $association_length and
        .request.frame.bytes[0:2] == [16, 0] and
        .request.frame.bytes[4:10] == [2, 170, 187, 204, 221, 1] and
        .request.frame.bytes[10:16] == $ap)
' "$chip_root/radio-replay.json" >/dev/null

jq -r '.uart[]' "$chip_root/result.json" | awk '{ printf "%c", $1 }' >"$chip_root/uart.log"
jq -r --arg chip "$chip" '.chips[$chip].required_uart_substrings[]' "$requirements" |
while IFS= read -r required_uart
do
    if ! grep -F "$required_uart" "$chip_root/uart.log" >/dev/null
    then
        echo "$chip vendor SoftAP/ESP-NOW milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

run_vendor_softap "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-wifi-softap-vendor-qualification.v1 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --argjson minimum_instructions "$minimum_instructions" \
    '{
        schema: $schema,
        chip: $chip,
        requirement: {
            real_rom_required: true,
            symbol_dispatch_allowed: false,
            deterministic_replay_required: true,
            minimum_instructions: $minimum_instructions
        },
        evidence: {
            rom_file: $rom_file,
            rom_sha256: $rom_sha256,
            elf_sha256: $elf_sha256,
            flash_sha256: $flash_sha256,
            uart_sha256: $uart_sha256,
            radio_replay_sha256: $radio_replay_sha256,
            native_softap_station_association: true,
            native_esp_now_tx_rx: true
        }
    }' >"$chip_root/qualification.json"

echo "$chip genuine SoftAP and ESP-NOW qualification passed"
