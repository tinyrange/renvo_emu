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

requirements=qualification/radio/ieee802154-vendor-requirements.json
artifact_root=${REMU_RADIO_IEEE802154_VENDOR_ROOT:-.remu/qualification/radio-ieee802154-vendor}
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
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/qualification/radio/sdkconfig.ieee802154.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig build && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-ieee802154-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-ieee802154-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_ieee802154()
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

run_vendor_ieee802154 "$chip_root/result.json" "$chip_root/radio-replay.json" \
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
        echo "$chip genuine IEEE 802.15.4 milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

jq -e '
    .events as $events |
    any($events[];
        .event == "submitted" and
        .request.frame.protocol == "ieee802154" and
        .request.frame.origin == "emulated" and
        .request.frame.spectrum.center_khz == 2405000 and
        .request.frame.bytes == [1, 0, 42, 165]) and
    any($events[];
        . as $submitted |
        ($submitted.event == "submitted" and
         $submitted.request.frame.protocol == "ieee802154" and
         $submitted.request.frame.origin == "host-injection" and
         $submitted.request.frame.bytes == [1, 0, 2, 170, 91, 37] and
         any($events[];
             .event == "reception" and
             .id == $submitted.id and
             .receiver == 1 and
             .outcome.kind == "delivered"))) and
    all([[1, 8, 49, 153, 153, 102, 102, 222, 173, 224, 158],
         [1, 8, 50, 205, 171, 87, 19, 222, 173, 54, 77],
         [33, 8, 70, 205, 171, 87, 19, 17, 34, 168, 241],
         [2, 0, 68, 152, 177]][];
        . as $bytes |
        any($events[];
            . as $submitted |
            ($submitted.event == "submitted" and
             $submitted.request.frame.protocol == "ieee802154" and
             $submitted.request.frame.origin == "host-injection" and
             $submitted.request.frame.bytes == $bytes and
             any($events[];
                 .event == "reception" and
                 .id == $submitted.id and
                 .receiver == 1 and
                 .outcome.kind == "delivered")))) and
    any($events[];
        . as $ack |
        ($ack.event == "submitted" and
         $ack.request.frame.origin == "emulated" and
         $ack.request.frame.bytes == [2, 0, 70, 138, 146] and
         ($ack.request.end - $ack.request.start) == 160 and
         any($events[];
             .event == "submitted" and
             .request.frame.origin == "host-injection" and
             .request.frame.bytes == [33, 8, 70, 205, 171, 87, 19, 17, 34, 168, 241] and
             .request.end == $ack.request.start))) and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        .request.frame.bytes == [33, 0, 68, 165]) and
    any($events[];
        .event == "submitted" and
        .request.frame.origin == "emulated" and
        .request.frame.bytes == [33, 0, 69, 165])
' "$chip_root/radio-replay.json" >/dev/null

# TX/RX completion causes 0x2 and 0x4 may be serviced separately or
# coalesced as 0x6 depending on the firmware instruction at which they latch.
# Require both causes and their W1C paths without prescribing ISR timing.
jq -e '
    any(.[]; .kind == "Write" and .address == 1611280384 and .value == 65) and
    any(.[]; .kind == "Write" and .address == 1611280384 and .value == 67) and
    any(.[]; .kind == "Write" and .address == 1611280384 and .value == 66) and
    any(.[]; .kind == "Write" and .address == 1611280384 and .value == 68) and
    any(.[]; .kind == "Write" and .address == 1611280592 and .value >= 1073741824) and
    any(.[]; .kind == "Write" and .address == 1611280608 and .value >= 1073741824) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 1) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 2) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 64) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 1) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 2) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 64) and
    any(.[]; .kind == "Write" and .address == 1611280388 and .value == 805306379) and
    any(.[]; .kind == "Write" and .address == 1611280392 and .value == 22136) and
    any(.[]; .kind == "Write" and .address == 1611280396 and .value == 4660) and
    any(.[]; .kind == "Write" and .address == 1611280408 and .value == 4951) and
    any(.[]; .kind == "Write" and .address == 1611280412 and .value == 43981) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 16) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 16) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 8) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 8) and
    any(.[]; .kind == "Read" and .address == 1611280484 and (.value == 4 or .value == 6)) and
    any(.[]; .kind == "Write" and .address == 1611280484 and (.value == 4 or .value == 6)) and
    any(.[]; .kind == "Write" and .address == 1611280492 and .value == 1) and
    any(.[]; .kind == "Write" and .address == 1611280384 and .value == 76) and
    any(.[]; .kind == "Read" and .address == 1611280484 and .value == 256) and
    any(.[]; .kind == "Write" and .address == 1611280484 and .value == 256)
' "$chip_root/ieee802154-bus.json" >/dev/null

run_vendor_ieee802154 "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
bus_sha=$(sha256sum "$chip_root/ieee802154-bus.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-ieee802154-vendor-qualification.v4 \
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
            requires_native_ieee802154_tx: true,
            requires_native_ieee802154_rx_dma: true,
            requires_fcs_validation: true,
            requires_rssi_lqi_metadata: true,
            requires_energy_detection: true,
            requires_cca_transmit: true,
            requires_multipan_filtering: true,
            requires_automatic_ack_transmit: true,
            requires_frame_pending_ack: true,
            requires_ack_success: true,
            requires_no_ack_timeout: true,
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
            tx_psdu_without_fcs: [1, 0, 42, 165],
            rx_psdu_with_fcs: [1, 0, 2, 170, 91, 37],
            rx_rssi_dbm: -80,
            rx_lqi: 63,
            cca_transmit: {
                duration_symbols: 8,
                clear_channel_completed: true
            },
            multipan_filter: {
                rejected_pan: 39321,
                accepted_pan: 43981,
                accepted_short_address: 4951,
                accepted_index: 1
            },
            automatic_ack_transmit: {
                received_sequence: 70,
                received_psdu_with_fcs: [33, 8, 70, 205, 171, 87, 19, 17, 34, 168, 241],
                transmitted_ack_with_fcs: [2, 0, 70, 138, 146],
                frame_pending: true
            },
            ack_success: {
                transmit_sequence: 68,
                received_psdu_with_fcs: [2, 0, 68, 152, 177]
            },
            no_ack_timeout: {
                transmit_sequence: 69,
                vendor_error: 3
            },
            deterministic_replay: true
        }
    }' >"$chip_root/summary.json"

echo "$chip genuine IEEE 802.15.4 qualification passed: $chip_root/summary.json"
