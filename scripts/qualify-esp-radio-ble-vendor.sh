#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

chip=${1:-}
case "$chip" in
    esp32c6|esp32s3) ;;
    *)
        echo "usage: $0 <esp32c6|esp32s3>" >&2
        exit 2
        ;;
esac

requirements=qualification/radio/ble-vendor-requirements.json
artifact_root=${REMU_RADIO_BLE_VENDOR_ROOT:-.remu/qualification/radio-ble-vendor}
vendor_build_jobs=${REMU_VENDOR_BUILD_JOBS:-1}
case "$vendor_build_jobs" in
    0|*[!0-9]*)
        echo "REMU_VENDOR_BUILD_JOBS must be a positive integer" >&2
        exit 2
        ;;
esac
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
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/qualification/radio/sdkconfig.ble.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig reconfigure && ninja -C /out/build -j $vendor_build_jobs && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-ble-scan-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-ble-scan-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_ble()
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

run_vendor_ble "$chip_root/result.json" "$chip_root/radio-replay.json" \
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

jq -r '.uart | implode' "$chip_root/result.json" >"$chip_root/uart.log"
jq -r --arg chip "$chip" '.chips[$chip].required_uart_substrings[]' "$requirements" |
while IFS= read -r required_uart
do
    if ! grep -F "$required_uart" "$chip_root/uart.log" >/dev/null
    then
        echo "$chip genuine BLE milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

jq -e --arg chip "$chip" --slurpfile requirements "$requirements" '
    $requirements[0].chips[$chip].expected_native_tx as $expected |
    $requirements[0].chips[$chip].expected_connection_tx as $expected_connection |
    .events as $events |
    any($events[];
        . as $submitted |
        ($submitted.event == "submitted" and
         $submitted.request.frame.protocol == "bluetooth-le" and
         $submitted.request.frame.origin == "host-injection" and
         $submitted.request.frame.bytes == [
             66, 12, 170, 187, 204, 221, 238, 193,
             2, 1, 6, 2, 9, 82
         ] and
         any($events[];
             .event == "reception" and
             .id == $submitted.id and
             .receiver == 1 and
             .outcome.kind == "delivered"))) and
    any($events[];
        . as $submitted |
        ($submitted.event == "submitted" and
         $submitted.request.frame.protocol == "bluetooth-le" and
         $submitted.request.frame.origin == "host-injection" and
         $submitted.request.frame.bytes[0:2] == [197, 34] and
         any($events[];
             .event == "reception" and
             .id == $submitted.id and
             .receiver == 1 and
             .outcome.kind == "delivered"))) and
    all([[15, 2, 2, 19],
         [2, 7, 3, 0, 4, 0, 2, 247, 0],
         [15, 9, 21, 251, 0, 144, 66, 251, 0, 144, 66]][];
        . as $required_rx |
        any($events[];
            . as $submitted |
            ($submitted.event == "submitted" and
             $submitted.request.frame.protocol == "bluetooth-le" and
             $submitted.request.frame.origin == "host-injection" and
             $submitted.request.frame.bytes == $required_rx and
             any($events[];
                 .event == "reception" and
                 .id == $submitted.id and
                 .receiver == 1 and
                 .outcome.kind == "delivered")))) and
    all([$expected.legacy, $expected.extended_primary, $expected.extended_auxiliary][];
        . as $required |
        any($events[];
            .event == "submitted" and
            .request.frame.protocol == "bluetooth-le" and
            .request.frame.origin == "emulated" and
            .request.frame.spectrum.center_khz == $required.center_khz and
            .request.frame.phy == $required.phy and
            .request.frame.bytes == $required.bytes)) and
    all($expected_connection[];
        . as $required |
        any($events[];
            .event == "submitted" and
            .request.frame.protocol == "bluetooth-le" and
            .request.frame.origin == "emulated" and
            .request.frame.spectrum.center_khz == $required.center_khz and
            .request.frame.phy == $required.phy and
            .request.frame.bytes == $required.bytes))
' "$chip_root/radio-replay.json" >/dev/null

run_vendor_ble "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-ble-vendor-qualification.v5 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --argjson entry "$entry" \
    --argjson minimum_instructions "$minimum_instructions" \
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
            requires_rom_execution: true,
            requires_vendor_firmware_execution: true,
            requires_native_ble_tx: true,
            requires_native_ble_stop: true,
            requires_native_ble_rx_ring: true,
            requires_native_ble_extended_advertising: true,
            requires_native_ble_auxiliary_tx: true,
            requires_native_ble_2m_phy: true,
            requires_native_ble_extended_scanning: true,
            requires_vendor_scan_report: true,
            requires_native_ble_connection_hopping: true,
            requires_native_ble_ll_version_exchange: true,
            requires_native_ble_feature_exchange: true,
            requires_native_ble_acl_rx: true,
            requires_native_ble_acl_tx: true,
            requires_native_ble_data_length_exchange: true,
            requires_vendor_mtu_callback: true,
            requires_native_ble_acl_acknowledgement: true,
            requires_native_ble_tx_descriptor_retirement: true,
            requires_native_ble_remote_termination: true,
            requires_native_ble_scan_restart: true,
            requires_received_power_metadata: true,
            deterministic_replay_required: true,
            symbol_dispatch_allowed: false
        },
        observed: {
            stop: $result[0].reason,
            instructions: $result[0].stats.instructions,
            unique_execute_addresses: $coverage[0].unique_addresses,
            coverage_digest: $coverage[0].digest,
            uart_sha256: $uart_sha256,
            radio_replay_sha256: $radio_replay_sha256,
            vendor_scan_payload: "02 01 06 02 09 52",
            vendor_scan_rssi_dbm: -80,
            native_extended_advertising: true,
            native_auxiliary_phy: "ble-2m",
            vendor_extended_scan_report: true,
            native_connection_established: true,
            native_data_length_octets: 251,
            vendor_att_mtu: 247,
            native_acl_acknowledged: true,
            native_connection_remote_terminated: true,
            native_scan_restarted_after_disconnect: true,
            deterministic_replay: true
        }
    }' >"$chip_root/summary.json"

echo "$chip genuine BLE qualification passed: $chip_root/summary.json"
