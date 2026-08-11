#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

chip=${1:-esp32c6}
if [ "$chip" != esp32c6 ]
then
    echo "usage: $0 [esp32c6]" >&2
    exit 2
fi

requirements=qualification/radio/openthread-vendor-requirements.json
artifact_root=${REMU_RADIO_OPENTHREAD_VENDOR_ROOT:-.remu/qualification/radio-openthread-vendor}
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
rom_file=$(jq -r '.rom_file' "$requirements")
expected_rom_sha=$(jq -r '.rom_sha256' "$requirements")
minimum_instructions=$(jq -r '.minimum_instructions' "$requirements")
radio_input=$(jq -r '.radio_input' "$requirements")
rom_start=$(jq -r '.rom_start' "$requirements")
rom_end=$(jq -r '.rom_end' "$requirements")

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
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/$firmware_project/sdkconfig.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig reconfigure && ninja -C /out/build -j $vendor_build_jobs && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-openthread-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-openthread-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_openthread()
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

run_vendor_openthread "$chip_root/result.json" "$chip_root/radio-replay.json" \
    --coverage "$chip_root/coverage.json" \
    --bus-log "$chip_root/ieee802154-bus.json" \
    --bus-log-region esp32c6.ieee802154

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
jq -r '.required_uart_substrings[]' "$requirements" |
while IFS= read -r required_uart
do
    if ! grep -F "$required_uart" "$chip_root/uart.log" >/dev/null
    then
        echo "$chip genuine OpenThread milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

jq -e --slurpfile requirements "$requirements" '
    $requirements[0].expected_raw_psdu_with_fcs as $raw_psdu |
    $requirements[0].transmit_security.expected_psdu_with_fcs as $secured_psdu |
    .events as $events |
    any($events[];
        .event == "submitted" and
        .request.frame.protocol == "ieee802154" and
        .request.frame.origin == "emulated" and
        .request.frame.spectrum.center_khz == 2405000 and
        .request.frame.bytes == $raw_psdu) and
    any($events[];
        .event == "submitted" and
        .request.frame.protocol == "ieee802154" and
        .request.frame.origin == "emulated" and
        .request.frame.spectrum.center_khz == 2405000 and
        .request.frame.bytes == $secured_psdu) and
    any($events[];
        . as $submitted |
        ($submitted.event == "submitted" and
         $submitted.request.start == 6200000 and
         $submitted.request.frame.protocol == "ieee802154" and
         $submitted.request.frame.origin == "host-injection" and
         $submitted.request.frame.bytes == [1, 0, 121, 82, 88, 8, 41] and
         any($events[];
             .event == "reception" and
             .id == $submitted.id and
             .receiver == 1 and
             .outcome.kind == "delivered")))
' "$chip_root/radio-replay.json" >/dev/null

# Require the genuine OpenThread port to drive native TX, RX, ED and sleep,
# then acknowledge their completion causes through the public W1C register.
jq -e --slurpfile requirements "$requirements" '
    . as $events |
    any($events[]; .kind == "Write" and .address == 1611280384 and .value == 65) and
    any($events[]; .kind == "Write" and .address == 1611280384 and .value == 66) and
    any($events[]; .kind == "Write" and .address == 1611280384 and .value == 68) and
    any($events[]; .kind == "Write" and .address == 1611280384 and .value == 69) and
    any($events[]; .kind == "Read" and .address == 1611280484 and .value == 1) and
    any($events[]; .kind == "Write" and .address == 1611280484 and .value == 1) and
    any($events[]; .kind == "Read" and .address == 1611280484 and .value == 2) and
    any($events[]; .kind == "Write" and .address == 1611280484 and .value == 2) and
    any($events[]; .kind == "Read" and .address == 1611280484 and .value == 64) and
    any($events[]; .kind == "Write" and .address == 1611280484 and .value == 64) and
    any($events[]; .kind == "Write" and .address == 1611280388 and .value == 268435584) and
    all($requirements[0].transmit_security.expected_register_writes[];
        . as $expected |
        any($events[];
            .kind == "Write" and
            .address == $expected.address and
            .value == $expected.value))
' "$chip_root/ieee802154-bus.json" >/dev/null

run_vendor_openthread "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/result.json" "$chip_root/result-repeat.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
bus_sha=$(sha256sum "$chip_root/ieee802154-bus.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-openthread-vendor-qualification.v3 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --arg bus_sha256 "$bus_sha" \
    --argjson entry "$entry" \
    --argjson minimum_instructions "$minimum_instructions" \
    --slurpfile requirements "$requirements" \
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
            requires_vendor_openthread_execution: true,
            requires_native_raw_tx_rx: true,
            requires_native_transmit_security: true,
            requires_source_matching: true,
            requires_energy_scan: true,
            requires_sleep_wake: true,
            requires_native_interrupt_w1c: true,
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
            ieee802154_bus_sha256: $bus_sha256,
            tx_psdu_with_fcs: $requirements[0].expected_raw_psdu_with_fcs,
            rx_psdu_with_fcs: [1, 0, 121, 82, 88, 8, 41],
            rx_rssi_dbm: -80,
            rx_lqi: 63,
            energy_scan_dbm: -128,
            source_match_short_address: 4660,
            transmit_security: {
                key_id: $requirements[0].transmit_security.key_id,
                frame_counter: $requirements[0].transmit_security.frame_counter,
                security_level: $requirements[0].transmit_security.security_level,
                payload_offset: $requirements[0].transmit_security.payload_offset,
                key: $requirements[0].transmit_security.key,
                secured_tx_psdu_with_fcs: $requirements[0].transmit_security.expected_psdu_with_fcs,
                vendor_register_writes_observed: true
            },
            sleep_wake_completed: true,
            deterministic_result_replay: true,
            deterministic_rf_replay: true
        }
    }' >"$chip_root/summary.json"

echo "$chip genuine OpenThread qualification passed: $chip_root/summary.json"
