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

requirements=qualification/radio/zigbee-vendor-requirements.json
artifact_root=${REMU_RADIO_ZIGBEE_VENDOR_ROOT:-.remu/qualification/radio-zigbee-vendor}
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

# Copy the small project into the artifact directory because IDF Component
# Manager materializes the pinned Apache-2.0 Zigbee component beside it.
docker run --rm \
    --user "$(id -u):$(id -g)" \
    --env HOME=/tmp \
    --volume "$repo_root:/workspace:ro" \
    --volume "$chip_root_absolute:/out" \
    "$idf_image" \
    bash -lc "cp -R /workspace/$firmware_project /out/project && IDF_TARGET=$chip idf.py -C /out/project -B /out/build -D SDKCONFIG=/out/sdkconfig reconfigure && ninja -C /out/build -j $vendor_build_jobs && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-zigbee-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

component_lock=$chip_root/project/dependencies.lock
expected_component_version=$(jq -r '.esp_zigbee.version' "$requirements")
expected_component_hash=$(jq -r '.esp_zigbee.component_hash' "$requirements")
locked_component_version=$(awk '
    /^  espressif\/esp-zigbee-lib:/ { component = 1; next }
    component && /^    version:/ { gsub(/["'\'' ]/, "", $2); print $2; exit }
' "$component_lock")
locked_component_hash=$(awk '
    /^  espressif\/esp-zigbee-lib:/ { component = 1; next }
    component && /^    component_hash:/ { print $2; exit }
' "$component_lock")
if [ "$locked_component_version" != "$expected_component_version" ] || \
   [ "$locked_component_hash" != "$expected_component_hash" ]
then
    echo "ESP Zigbee component lock mismatch" >&2
    exit 1
fi
if grep -i 'zboss' "$component_lock" >/dev/null || \
   find "$chip_root/project/managed_components" -maxdepth 1 -type d \
       -iname '*zboss*' -print -quit | grep . >/dev/null
then
    echo "retired ZBOSS dependency is not permitted" >&2
    exit 1
fi

component_root=$chip_root/project/managed_components/espressif__esp-zigbee-lib
expected_license_sha=$(jq -r '.esp_zigbee.license_sha256' "$requirements")
actual_license_sha=$(sha256sum "$component_root/LICENSE" | cut -d ' ' -f 1)
if [ "$actual_license_sha" != "$expected_license_sha" ]
then
    echo "ESP Zigbee component license hash mismatch" >&2
    exit 1
fi
jq -r '.esp_zigbee.library_sha256 | to_entries[] | [.key, .value] | @tsv' \
    "$requirements" |
while IFS="$(printf '\t')" read -r library expected_sha
do
    actual_sha=$(sha256sum "$component_root/lib/$chip/$library" | cut -d ' ' -f 1)
    if [ "$actual_sha" != "$expected_sha" ]
    then
        echo "ESP Zigbee library hash mismatch for $library" >&2
        exit 1
    fi
done

if [ -z "${REMU_BIN:-}" ]
then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-zigbee-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_zigbee()
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

run_vendor_zigbee "$chip_root/result.json" "$chip_root/radio-replay.json" \
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
        echo "$chip genuine Zigbee milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

jq -e --slurpfile requirements "$requirements" '
    .events as $events |
    $requirements[0].expected_host_beacon_request as $host_bytes |
    [ $events[] |
        select(.event == "submitted" and
               .request.frame.protocol == "ieee802154" and
               .request.frame.origin == "emulated") |
        {start: .request.start, end: .request.end, bytes: .request.frame.bytes} ] as $emulated |
    [ $emulated[] |
        select(.bytes[0:2] == $requirements[0].expected_data_frame_control) ] as $data |
    [ $data[] |
        select((.bytes | length) == $requirements[0].expected_formation_data_shape.length and
               .bytes[9:11] == $requirements[0].expected_formation_data_shape.nwk_control_le) ] as $formation |
    [ $data[] |
        . as $frame |
        select(any($requirements[0].allowed_optional_data_shapes[];
            (.length == ($frame.bytes | length)) and
            (.nwk_control_le == $frame.bytes[9:11]))) ] as $optional |
    [ $emulated[] |
        select((.bytes | length) == $requirements[0].expected_beacon_response_length and
               .bytes[0:2] == $requirements[0].expected_beacon_response_control and
               .bytes[3:-2] == $requirements[0].expected_beacon_response_body) ] as $beacons |
    (($emulated | length) >= $requirements[0].expected_minimum_emulated_frames) and
    (($emulated | length) <= $requirements[0].expected_maximum_emulated_frames) and
    (($emulated[0].bytes | length) == $requirements[0].expected_beacon_request_length) and
    ($emulated[0].bytes[0:2] == $requirements[0].expected_beacon_request_control) and
    ($emulated[0].bytes[3:8] ==
        $requirements[0].expected_beacon_request_body) and
    all(range(1; $emulated | length);
        . as $index |
        ($emulated[$index].bytes[3:5] == $requirements[0].expected_pan_id_le) and
        (($emulated[$index].bytes[0:2] == $requirements[0].expected_data_frame_control) or
         ($emulated[$index].bytes[0:2] == $requirements[0].expected_beacon_response_control))) and
    (($formation | length) == $requirements[0].expected_formation_data_shape.count) and
    (($optional | length) <=
        ([ $requirements[0].allowed_optional_data_shapes[].maximum_count ] | add)) and
    (($data | length) == (($formation | length) + ($optional | length))) and
    (($beacons | length) == 1) and
    any($events[];
        . as $host |
        ($host.event == "submitted" and
         $host.request.start == 12000000 and
         $host.request.frame.origin == "host-injection" and
         $host.request.frame.bytes == $host_bytes and
         any($events[];
             .event == "reception" and
             .id == $host.id and
             .receiver == 1 and
             .outcome.kind == "delivered") and
         ($formation[0].start < $host.request.start) and
         ($beacons[0].start >= $host.request.end) and
         ($formation[1].start > $beacons[0].end))) and
    ([.coexistence_events[] | select(.event == "granted" and .protocol == "ieee802154")] | length)
        == ($emulated | length)
' "$chip_root/radio-replay.json" >/dev/null

python3 - "$chip_root/radio-replay.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    replay = json.load(source)
frames = [
    event["request"]["frame"]["bytes"]
    for event in replay["events"]
    if event["event"] == "submitted"
    and event["request"]["frame"]["protocol"] == "ieee802154"
]
for index, frame in enumerate(frames):
    crc = 0
    for byte in frame[:-2]:
        value = byte
        for _ in range(8):
            mix = (crc ^ value) & 1
            crc >>= 1
            if mix:
                crc ^= 0x8408
            value >>= 1
    if frame[-2:] != [crc & 0xff, crc >> 8]:
        raise SystemExit(f"IEEE 802.15.4 frame {index} has invalid hardware FCS")
PY

jq -e --slurpfile requirements "$requirements" '
    . as $events |
    all($requirements[0].required_commands[];
        . as $command |
        any($events[];
            .kind == "Write" and .address == 1611280384 and .value == $command)) and
    all($requirements[0].required_interrupt_causes[];
        . as $cause |
        any($events[];
            .kind == "Read" and .address == 1611280484 and .value == $cause) and
        any($events[];
            .kind == "Write" and .address == 1611280484 and .value == $cause))
' "$chip_root/ieee802154-bus.json" >/dev/null

run_vendor_zigbee "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/result.json" "$chip_root/result-repeat.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
bus_sha=$(sha256sum "$chip_root/ieee802154-bus.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-zigbee-vendor-qualification.v1 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg component_version "$expected_component_version" \
    --arg component_hash "$expected_component_hash" \
    --arg component_license_sha256 "$actual_license_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --arg bus_sha256 "$bus_sha" \
    --argjson entry "$entry" \
    --argjson minimum_instructions "$minimum_instructions" \
    --slurpfile requirements "$requirements" \
    --slurpfile result "$chip_root/result.json" \
    --slurpfile radio_replay "$chip_root/radio-replay.json" \
    --slurpfile coverage "$chip_root/coverage.json" \
    '{
        schema: $schema,
        chip: $chip,
        requirement: {
            rom_file: $rom_file,
            rom_sha256: $rom_sha256,
            esp_zigbee_version: $component_version,
            esp_zigbee_component_hash: $component_hash,
            esp_zigbee_license_sha256: $component_license_sha256,
            firmware_elf_sha256: $elf_sha256,
            firmware_flash_sha256: $flash_sha256,
            application_entry: $entry,
            minimum_instructions: $minimum_instructions,
            requires_rom_execution: true,
            requires_vendor_zigbee_execution: true,
            requires_native_network_formation: true,
            requires_peer_beacon_exchange: true,
            requires_hardware_generated_fcs: true,
            requires_native_cca_ed_rx_tx_interrupts: true,
            deterministic_replay_required: true,
            symbol_dispatch_allowed: false,
            zboss_dependency_allowed: false
        },
        observed: {
            stop: $result[0].reason,
            instructions: $result[0].stats.instructions,
            unique_execute_addresses: $coverage[0].unique_addresses,
            coverage_digest: $coverage[0].digest,
            uart_sha256: $uart_sha256,
            radio_replay_sha256: $radio_replay_sha256,
            ieee802154_bus_sha256: $bus_sha256,
            pan_id: 63100,
            channel: 11,
            short_address: 0,
            injected_beacon_request: $requirements[0].expected_host_beacon_request,
            expected_frame_contract: {
                minimum_frames: $requirements[0].expected_minimum_emulated_frames,
                maximum_frames: $requirements[0].expected_maximum_emulated_frames,
                beacon_request_control: $requirements[0].expected_beacon_request_control,
                beacon_request_length: $requirements[0].expected_beacon_request_length,
                data_control: $requirements[0].expected_data_frame_control,
                beacon_response_control: $requirements[0].expected_beacon_response_control,
                beacon_response_length: $requirements[0].expected_beacon_response_length,
                formation_data: $requirements[0].expected_formation_data_shape,
                allowed_optional_data: $requirements[0].allowed_optional_data_shapes
            },
            observed_emulated_frame_shapes: [
                $radio_replay[0].events[] |
                select(.event == "submitted" and
                       .request.frame.protocol == "ieee802154" and
                       .request.frame.origin == "emulated") |
                {
                    length: (.request.frame.bytes | length),
                    mac_control: .request.frame.bytes[0:2],
                    nwk_control: .request.frame.bytes[9:11]
                }
            ],
            observed_emulated_frames: [
                $radio_replay[0].events[] |
                select(.event == "submitted" and
                       .request.frame.protocol == "ieee802154" and
                       .request.frame.origin == "emulated") |
                .request.frame.bytes
            ],
            deterministic_result_replay: true,
            deterministic_rf_replay: true
        }
    }' >"$chip_root/summary.json"

echo "$chip genuine Zigbee qualification passed: $chip_root/summary.json"
