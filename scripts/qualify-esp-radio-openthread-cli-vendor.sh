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

requirements=qualification/radio/openthread-cli-vendor-requirements.json
artifact_root=${REMU_RADIO_OPENTHREAD_CLI_VENDOR_ROOT:-.remu/qualification/radio-openthread-cli-vendor}
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
rom_start=$(jq -r '.rom_start' "$requirements")
rom_end=$(jq -r '.rom_end' "$requirements")
radio_script=qualification/radio/openthread-cli-peer-esp32c6.star

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
    bash -lc "IDF_TARGET=$chip SDKCONFIG_DEFAULTS=/workspace/$firmware_project/sdkconfig.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig reconfigure && ninja -C /out/build -j $vendor_build_jobs && cd /out/build && python -m esptool --chip $chip merge-bin -o /out/$chip-openthread-cli-flash.bin @flash_args" \
    >"$chip_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

elf=$build_root/$firmware_elf
flash=$chip_root/$chip-openthread-cli-flash.bin
entry_hex=$(readelf -h "$elf" | awk '/Entry point address:/ { print $4 }')
entry=$((entry_hex))
rom_start_decimal=$((rom_start))
rom_end_decimal=$((rom_end))

run_vendor_openthread_cli()
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
        --radio-script "$radio_script" \
        --radio-replay "$replay" \
        --result "$result" \
        "$@"
}

run_vendor_openthread_cli "$chip_root/result.json" "$chip_root/radio-replay.json" \
    --coverage "$chip_root/coverage.json" \
    --bus-log "$chip_root/peripheral-bus.json" \
    --bus-log-region esp32c6.ieee802154 \
    --bus-log-region esp32c6.aes \
    --bus-log-region esp32c6.spimem1

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
        echo "$chip full-stack OpenThread milestone missing from UART: $required_uart" >&2
        exit 1
    fi
done

jq -e --slurpfile requirements "$requirements" '
    .events as $events |
    [.events[] |
        select(.event == "submitted" and
               .request.frame.protocol == "ieee802154" and
               .request.frame.origin == "emulated" and
               (.request.frame.bytes | length) > 5 and
               .request.frame.spectrum.center_khz == 2405000 and
               .request.frame.bytes[3:5] == [52, 18])] as $frames |
    ($frames | length) >= $requirements[0].minimum_thread_frames and
    any($frames[]; .request.frame.bytes[0:2] == [65, 216] and
                   (.request.frame.bytes | length) >= 70) and
    any($frames[]; .request.frame.bytes[0:2] == [105, 220] and
                   (.request.frame.bytes | length) == 60 and
                   .request.frame.bytes[21:27] == [13, 2, 0, 0, 0, 1]) and
    any(.events[]; .event == "submitted" and
        .request.frame.origin == "host-injection" and
        .request.frame.bytes[0:3] == [105, 220, 96] and
        .request.frame.bytes[21:27] == [13, 3, 0, 0, 0, 1]) and
    any(.events[]; .event == "reception" and .outcome.kind == "delivered") and
    ([.coexistence_events[] |
        select(.event == "granted" and .protocol == "ieee802154")] | length)
        == ($frames | length)
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
        raise SystemExit(f"Thread frame {index} has invalid hardware FCS")
PY

# Require native CCA/TX/RX/stop commands and their W1C completion, the
# non-DMA AES trigger/idle contract used by PSA, and real SPI1 flash
# erase/program/read commands used by NVS-backed persistent keys.
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
            .kind == "Write" and .address == 1611280484 and .value == $cause)) and
    any($events[]; .kind == "Write" and .address == 1611169864 and .value == 1) and
    any($events[]; .kind == "Read" and .address == 1611169868 and .value == 0) and
    any($events[]; .kind == "Write" and .address == 1610625056 and .value == 1879048194) and
    any($events[]; .kind == "Write" and .address == 1610625056 and .value == 1879048224) and
    any($events[]; .kind == "Write" and .address == 1610625056 and .value == 1879048379)
' "$chip_root/peripheral-bus.json" >/dev/null

run_vendor_openthread_cli "$chip_root/result-repeat.json" "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json"
cmp "$chip_root/result.json" "$chip_root/result-repeat.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"

elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$flash" | cut -d ' ' -f 1)
uart_sha=$(sha256sum "$chip_root/uart.log" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
radio_script_sha=$(sha256sum "$radio_script" | cut -d ' ' -f 1)
bus_sha=$(sha256sum "$chip_root/peripheral-bus.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-openthread-cli-vendor-qualification.v1 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg flash_sha256 "$flash_sha" \
    --arg uart_sha256 "$uart_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --arg radio_script_sha256 "$radio_script_sha" \
    --arg bus_sha256 "$bus_sha" \
    --argjson entry "$entry" \
    --argjson minimum_instructions "$minimum_instructions" \
    --slurpfile requirements "$requirements" \
    --slurpfile result "$chip_root/result.json" \
    --slurpfile coverage "$chip_root/coverage.json" \
    --slurpfile replay "$chip_root/radio-replay.json" \
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
            requires_vendor_openthread_ftd: true,
            requires_default_platform_key_references: true,
            requires_nvs_psa_persistence: true,
            requires_cli_dataset_and_ping: true,
            requires_event_driven_starlark_peer: true,
            radio_script_sha256: $radio_script_sha256,
            requires_authenticated_parent_child_attach: true,
            requires_protected_unicast_echo: true,
            requires_native_thread_tx: true,
            requires_native_aes: true,
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
            peripheral_bus_sha256: $bus_sha256,
            thread_frames: ([$replay[0].events[] |
                select(.event == "submitted" and
                       .request.frame.protocol == "ieee802154")] | length),
            leader_role: 4,
            child_attached: true,
            protected_echo_reply: true,
            pan_id: $requirements[0].expected_pan_id,
            nvs_round_trip: true,
            psa_persistent_key_import: true,
            non_dma_aes_returns_idle: true,
            spi_flash_erase_program_read: true,
            deterministic_result_replay: true,
            deterministic_rf_replay: true
        }
    }' >"$chip_root/summary.json"

echo "$chip genuine full-stack OpenThread CLI qualification passed: $chip_root/summary.json"
