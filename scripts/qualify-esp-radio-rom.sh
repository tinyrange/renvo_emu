#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

chip=${1:-}
case "$chip" in
    esp32c6|esp32s3) ;;
    *) echo "usage: $0 esp32c6|esp32s3" >&2; exit 2 ;;
esac

requirements=qualification/radio/rom-requirements.json
artifact_root=${REMU_RADIO_ROM_ROOT:-.remu/qualification/radio-rom}
chip_root=$artifact_root/$chip
rom_root=$chip_root/roms
build_root=$chip_root/build
idf_image=$(jq -r '.esp_idf_container' "$requirements")
firmware_project=$(jq -r '.firmware_project' "$requirements")
firmware_elf=$(jq -r '.firmware_elf' "$requirements")
rom_file=$(jq -r --arg chip "$chip" '.chips[$chip].rom_file' "$requirements")
expected_rom_sha=$(jq -r --arg chip "$chip" '.chips[$chip].rom_sha256' "$requirements")
minimum_instructions=$(jq -r --arg chip "$chip" '.chips[$chip].minimum_instructions' "$requirements")
minimum_wifi_tx_frames=$(jq -r --arg chip "$chip" '.chips[$chip].minimum_wifi_tx_frames' "$requirements")
radio_input=$(jq -r --arg chip "$chip" '.chips[$chip].radio_input' "$requirements")
rom_start=$(jq -r --arg chip "$chip" '.chips[$chip].rom_start' "$requirements")
rom_end=$(jq -r --arg chip "$chip" '.chips[$chip].rom_end' "$requirements")

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
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/qualification/radio/sdkconfig.rom.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig build && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-wifi-scan-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-wifi-scan-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

"$remu" run \
    --target "$chip" \
    --elf "$elf" \
    --boot-rom "$rom_root/$rom_file" \
    --esp-app-image "$flash" \
    --max-instructions "$minimum_instructions" \
    --coverage "$chip_root/coverage.json" \
    --radio-input "$radio_input" \
    --radio-replay "$chip_root/radio-replay.json" \
    --result "$chip_root/result.json"

jq -e --argjson instructions "$minimum_instructions" \
    '.reason == "InstructionLimit" and .stats.instructions == $instructions' \
    "$chip_root/result.json" >/dev/null
jq -e --argjson start "$rom_start_decimal" --argjson end "$rom_end_decimal" \
    'any(.addresses[]; .address >= $start and .address < $end)' \
    "$chip_root/coverage.json" >/dev/null
jq -e --argjson entry "$entry" \
    'any(.addresses[]; .address == $entry)' \
    "$chip_root/coverage.json" >/dev/null
jq -e --argjson minimum "$minimum_wifi_tx_frames" '
    [.events[] |
        select(.event == "submitted" and
               .request.frame.protocol == "wifi" and
               .request.frame.origin == "emulated" and
               (.request.frame.bytes | length) >= 24 and
               .request.frame.bytes[0:2] == [64, 0])] |
    length >= $minimum
' "$chip_root/radio-replay.json" >/dev/null
jq -e '
    any(.events[];
        .event == "submitted" and
        .request.frame.protocol == "wifi" and
        .request.frame.origin == "host-injection") and
    any(.events[];
        .event == "reception" and
        .receiver == 1 and
        .outcome.kind == "delivered")
' "$chip_root/radio-replay.json" >/dev/null

jq -r '.uart[]' "$chip_root/result.json" | awk '{ printf "%c", $1 }' >"$chip_root/uart.log"
jq -r --arg chip "$chip" '.chips[$chip].required_uart_substrings[]' "$requirements" |
while IFS= read -r required_uart
do
    if ! grep -F "$required_uart" "$chip_root/uart.log" >/dev/null
    then
        echo "$chip vendor radio-init milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done
scan_count=$(sed -n \
    's/.*REMU_VENDOR_WIFI_SCAN_DONE result=0 count=\([0-9][0-9]*\).*/\1/p' \
    "$chip_root/uart.log" | tail -n 1)
if [ -z "$scan_count" ] || [ "$scan_count" -lt 1 ]
then
    echo "$chip vendor firmware did not report the injected access point" >&2
    exit 1
fi

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-rom-qualification.v1 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --argjson entry "$entry" \
    --argjson minimum_instructions "$minimum_instructions" \
    --argjson minimum_wifi_tx_frames "$minimum_wifi_tx_frames" \
    --argjson vendor_scan_count "$scan_count" \
    --slurpfile result "$chip_root/result.json" \
    --slurpfile coverage "$chip_root/coverage.json" \
    '{
        schema: $schema,
        chip: $chip,
        requirement: {
            rom_file: $rom_file,
            rom_sha256: $rom_sha256,
            firmware_elf_sha256: $elf_sha256,
            firmware_flash_sha256: $flash_sha256,
            application_entry: $entry,
            minimum_instructions: $minimum_instructions,
            required_stop: "InstructionLimit",
            requires_rom_execution: true,
            requires_application_entry: true,
            requires_vendor_firmware_execution: true,
            requires_radio_initialization: true,
            requires_native_wifi_dma_tx: true,
            requires_native_wifi_dma_rx: true,
            requires_vendor_scan_result: true,
            minimum_wifi_tx_frames: $minimum_wifi_tx_frames,
            symbol_dispatch_allowed: false
        },
        observed: {
            stop: $result[0].reason,
            instructions: $result[0].stats.instructions,
            unique_execute_addresses: $coverage[0].unique_addresses,
            coverage_digest: $coverage[0].digest,
            uart_sha256: $uart_sha256,
            radio_replay_sha256: $radio_replay_sha256,
            vendor_scan_count: $vendor_scan_count
        }
    }' >"$chip_root/summary.json"

echo "$chip real-ROM qualification passed: $chip_root/summary.json"
