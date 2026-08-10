#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

chip=${1:-}
case "$chip" in
    esp32c6|esp32s3) ;;
    *) echo "usage: $0 esp32c6|esp32s3" >&2; exit 2 ;;
esac

requirements=qualification/radio/custom-stack-probe/requirements.json
rom_requirements=qualification/radio/rom-requirements.json
source_root=qualification/radio/custom-stack-probe
radio_input=qualification/radio/wifi-ieee802154-custom.json
if [ "$chip" = esp32s3 ]
then
    radio_input=qualification/radio/wifi-beacon-ble-advertisement-custom.json
fi
artifact_root=${REMU_RADIO_CUSTOM_ROOT:-.remu/qualification/radio-custom-stack}
chip_root=$artifact_root/$chip
out=$chip_root/out
source=$(jq -r --arg chip "$chip" '.chips[$chip].source' "$requirements")
toolchain=$(jq -r --arg chip "$chip" '.chips[$chip].toolchain' "$requirements")
artifact=$(jq -r --arg chip "$chip" '.chips[$chip].artifact' "$requirements")
max_instructions=$(jq -r --arg chip "$chip" '.chips[$chip].max_instructions' "$requirements")
rom_release=$(jq -r '.rom_release' "$rom_requirements")
rom_file=$(jq -r --arg chip "$chip" '.chips[$chip].rom_file' "$rom_requirements")
expected_rom_sha=$(jq -r --arg chip "$chip" '.chips[$chip].rom_sha256' "$rom_requirements")
rom_root=${REMU_ESP_ROM_DIR:-.remu/qualification/esp-rom-elfs/$rom_release}
rom=$rom_root/$rom_file

if [ ! -f "$rom" ] || [ "$(sha256sum "$rom" | cut -d ' ' -f 1)" != "$expected_rom_sha" ]
then
    REMU_ESP_ROM_DIR=$rom_root scripts/fetch-esp-rom-elfs.sh >/dev/null
fi
actual_rom_sha=$(sha256sum "$rom" | cut -d ' ' -f 1)
if [ "$actual_rom_sha" != "$expected_rom_sha" ]
then
    echo "$chip mask-ROM hash mismatch: expected $expected_rom_sha, got $actual_rom_sha" >&2
    exit 1
fi

rm -rf "$chip_root"
mkdir -p "$out"

if [ -z "${REMU_BIN:-}" ]
then
    cargo build -q -p remu-cli --locked
    remu=target/debug/remu
else
    remu=$REMU_BIN
fi

case "$chip" in
    esp32c6)
        build_inputs="start-esp32c6.S $source -Wl,-T,link-esp32c6.ld"
        ;;
    esp32s3)
        build_inputs="$source -Wl,-T,link-esp32s3.ld"
        ;;
esac

# shellcheck disable=SC2086 -- build_inputs is an intentional compiler argv list.
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source "$source_root" \
    --output "$out" \
    --target "$chip" \
    --artifact "$chip_root/build.json" \
    -- -O2 $build_inputs -o "/workspace/out/$artifact"

"$remu" run \
    --target "$chip" \
    --elf "$out/$artifact" \
    --boot-rom "$rom" \
    --max-instructions "$max_instructions" \
    --bus-log "$chip_root/bus.json" \
    --coverage "$chip_root/coverage.json" \
    --radio-input "$radio_input" \
    --radio-replay "$chip_root/radio-replay.json" \
    --result "$chip_root/result.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$chip_root/result.json" >/dev/null
jq -e '
    any(.events[];
        .event == "submitted" and
        .request.frame.protocol == "wifi" and
        .request.frame.origin == "emulated" and
        (.request.frame.bytes | length) == 24 and
        .request.frame.bytes[0:4] == [64, 0, 0, 0])
' "$chip_root/radio-replay.json" >/dev/null
if [ "$chip" = esp32s3 ]
then
    jq -e '
        any(.events[];
            .event == "submitted" and
            .request.frame.protocol == "bluetooth-le" and
            .request.frame.origin == "emulated" and
            .request.frame.spectrum.center_khz == 2480000 and
            .request.frame.bytes == [
                70, 21, 2, 17, 34, 51, 68, 85,
                2, 1, 6, 11, 9, 82, 101, 110, 118, 111, 45, 66, 76, 69, 49
            ])
    ' "$chip_root/radio-replay.json" >/dev/null
    jq -e '
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
                .outcome.kind == "delivered")))
    ' "$chip_root/radio-replay.json" >/dev/null
fi
if [ "$chip" = esp32c6 ]
then
	    jq -e '
	        .events as $events |
	        any($events[];
	            .event == "submitted" and
	            .request.frame.protocol == "ieee802154" and
	            .request.frame.origin == "emulated" and
	            .request.frame.bytes == [1, 0, 42, 165]) and
	        any($events[];
	            .event == "submitted" and
	            .request.frame.protocol == "ieee802154" and
	            .request.frame.origin == "emulated" and
	            .request.frame.bytes == [33, 0, 68, 165]) and
	        any($events[];
	            .event == "submitted" and
	            .request.frame.protocol == "ieee802154" and
	            .request.frame.origin == "emulated" and
	            .request.frame.bytes == [33, 0, 69, 165]) and
	        any($events[];
	            .event == "submitted" and
	            .request.frame.protocol == "ieee802154" and
	            .request.frame.origin == "emulated" and
	            .request.frame.bytes == [2, 0, 70, 138, 146]) and
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
	             [2, 0, 68, 152, 177],
	             [33, 8, 70, 205, 171, 87, 19, 17, 34, 168, 241]][];
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
            .event == "submitted" and
            .request.frame.protocol == "bluetooth-le" and
            .request.frame.origin == "emulated" and
            .request.frame.spectrum.center_khz == 2402000 and
            .request.frame.bytes == [
                70, 21, 2, 0, 0, 0, 0, 198,
                2, 1, 6, 11, 9, 82, 101, 110, 118, 111, 45, 66, 76, 69, 49
            ]) and
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
                 .outcome.kind == "delivered")))
    ' "$chip_root/radio-replay.json" >/dev/null
fi
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

"$remu" run \
    --target "$chip" \
    --elf "$out/$artifact" \
    --boot-rom "$rom" \
    --max-instructions "$max_instructions" \
    --radio-input "$radio_input" \
    --radio-replay "$chip_root/radio-replay-repeat.json" \
    --replay "$chip_root/result.json" \
    --result "$chip_root/result-repeat.json"
cmp "$chip_root/radio-replay.json" "$chip_root/radio-replay-repeat.json"
jq -r --arg chip "$chip" '.chips[$chip].required_regions[]' "$requirements" |
while IFS= read -r region
do
    if ! jq -e --arg region "$region" 'any(.[]; .region == $region)' "$chip_root/bus.json" >/dev/null
    then
        echo "$chip custom radio firmware did not access required region: $region" >&2
        exit 1
    fi
done

descriptor_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_wifi_descriptor" { print $1; exit }')
frame_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_wifi_frame" { print $1; exit }')
rx_descriptor_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_wifi_rx_descriptor" { print $1; exit }')
rx_buffer_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_wifi_rx_buffer" { print $1; exit }')
ble_rx_descriptor_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_ble_rx_descriptor" { print $1; exit }')
ble_rx_payload_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_ble_rx_payload" { print $1; exit }')
ieee802154_rx_buffer_hex=$(nm -g --defined-only "$out/$artifact" |
    awk '$3 == "remu_ieee802154_rx_buffer" { print $1; exit }')
if [ -z "$descriptor_hex" ] || [ -z "$frame_hex" ] ||
    [ -z "$rx_descriptor_hex" ] || [ -z "$rx_buffer_hex" ]
then
    echo "$chip custom radio firmware is missing DMA evidence symbols" >&2
    exit 1
fi
if [ -z "$ble_rx_descriptor_hex" ] || [ -z "$ble_rx_payload_hex" ]
then
    echo "$chip custom radio firmware is missing native BLE RX evidence symbols" >&2
    exit 1
fi
if [ "$chip" = esp32c6 ] && [ -z "$ieee802154_rx_buffer_hex" ]
then
    echo "$chip custom radio firmware is missing native IEEE 802.15.4 RX evidence symbol" >&2
    exit 1
fi
descriptor_address=$((0x$descriptor_hex))
frame_address=$((0x$frame_hex))
rx_descriptor_address=$((0x$rx_descriptor_hex))
rx_buffer_address=$((0x$rx_buffer_hex))
ble_rx_descriptor_address=0
ble_rx_payload_address=0
ieee802154_rx_buffer_address=0
ble_rx_descriptor_address=$((0x$ble_rx_descriptor_hex))
ble_rx_payload_address=$((0x$ble_rx_payload_hex))
if [ "$chip" = esp32c6 ]
then
    ieee802154_rx_buffer_address=$((0x$ieee802154_rx_buffer_hex))
fi
frame_data_offset=$(jq -r --arg chip "$chip" \
    '.chips[$chip].wifi_frame_data_offset' "$requirements")
frame_data_address=$((frame_address + frame_data_offset))
jq -e --argjson descriptor "$descriptor_address" \
    --argjson frame "$frame_data_address" \
    --argjson rx_descriptor "$rx_descriptor_address" \
    --argjson rx_buffer "$rx_buffer_address" '
    any(.[]; .kind == "Read" and .address == ($descriptor + 4)) and
    any(.[]; .kind == "Read" and .address == $frame) and
    any(.[]; .kind == "Write" and .address == $rx_descriptor) and
    any(.[]; .kind == "Write" and .address == $rx_buffer)
' "$chip_root/bus.json" >/dev/null
if [ "$chip" = esp32s3 ]
then
    jq -e --argjson descriptor "$ble_rx_descriptor_address" \
        --argjson payload "$ble_rx_payload_address" '
        any(.[];
            .kind == "Write" and
            .address >= $descriptor and
            .address < ($descriptor + 20)) and
        any(.[];
            .kind == "Write" and
            .address >= $payload and
            .address < ($payload + 12))
    ' "$chip_root/bus.json" >/dev/null
fi
if [ "$chip" = esp32c6 ]
then
	    jq -e --argjson buffer "$ieee802154_rx_buffer_address" '
        any(.[];
            .kind == "Write" and
            .address >= $buffer and
            .address < ($buffer + 5))
	    ' "$chip_root/bus.json" >/dev/null
	    jq -e '
	        any(.[]; .kind == "Write" and .address == 1611280388 and .value == 805306376) and
	        any(.[]; .kind == "Write" and .address == 1611280388 and .value == 805306377) and
	        any(.[]; .kind == "Write" and .address == 1611280392 and .value == 22136) and
	        any(.[]; .kind == "Write" and .address == 1611280396 and .value == 4660) and
	        any(.[]; .kind == "Write" and .address == 1611280408 and .value == 4951) and
	        any(.[]; .kind == "Write" and .address == 1611280412 and .value == 43981) and
	        any(.[]; .kind == "Read" and .address == 1611280484 and .value == 16) and
	        any(.[]; .kind == "Write" and .address == 1611280484 and .value == 16) and
	        any(.[]; .kind == "Read" and .address == 1611280484 and .value == 8) and
	        any(.[]; .kind == "Write" and .address == 1611280484 and .value == 8) and
	        any(.[]; .kind == "Write" and .address == 1611280552 and .value == 512) and
	        any(.[]; .kind == "Write" and .address == 1611280384 and .value == 76) and
	        any(.[]; .kind == "Write" and .address == 1611280680 and .value == 1281) and
	        any(.[]; .kind == "Read" and .address == 1611280516 and .value == 65840) and
	        any(.[]; .kind == "Read" and .address == 1611280760 and .value == 1) and
	        any(.[]; .kind == "Write" and .address == 1611280464 and .value == 8) and
	        any(.[]; .kind == "Write" and .address == 1611280468 and .value == 16565) and
	        any(.[]; .kind == "Write" and .address == 1611280384 and .value == 67) and
	        any(.[]; .kind == "Read" and .address == 1611280484 and .value == 256) and
	        any(.[]; .kind == "Write" and .address == 1611280484 and .value == 256)
	        and any(.[]; .kind == "Read" and .address == 1611280484 and .value == 6)
	        and any(.[]; .kind == "Write" and .address == 1611280484 and .value == 6)
	    ' "$chip_root/bus.json" >/dev/null
    jq -e --argjson payload "$ble_rx_payload_address" '
        any(.[];
            .kind == "Write" and
            .address == ($payload + 12)) and
        any(.[];
            .kind == "Write" and
            .address == ($payload + 28)) and
        any(.[];
            .kind == "Read" and
            .address == 1611272972 and
            .value == 404750336) and
        any(.[];
            .kind == "Write" and
            .address == 1611272968 and
            .value == 404750336) and
        any(.[];
            .kind == "Read" and
            .address == 1610678580 and
            ((((.value / 32) | floor) % 2) == 1)) and
        any(.[];
            .kind == "Read" and
            .address == 536875020 and
            ((((.value / 256) | floor) % 2) == 1))
    ' "$chip_root/bus.json" >/dev/null
fi

elf_sha=$(sha256sum "$out/$artifact" | cut -d ' ' -f 1)
radio_replay_sha=$(sha256sum "$chip_root/radio-replay.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.radio-custom-stack-qualification.v3 \
    --arg chip "$chip" \
    --arg rom_file "$rom_file" \
    --arg rom_sha256 "$actual_rom_sha" \
    --arg elf_sha256 "$elf_sha" \
    --arg radio_replay_sha256 "$radio_replay_sha" \
    --argjson descriptor_address "$descriptor_address" \
    --argjson frame_data_address "$frame_data_address" \
    --argjson rx_descriptor_address "$rx_descriptor_address" \
    --argjson rx_buffer_address "$rx_buffer_address" \
    --argjson ble_rx_descriptor_address "$ble_rx_descriptor_address" \
    --argjson ble_rx_payload_address "$ble_rx_payload_address" \
    --argjson ieee802154_rx_buffer_address "$ieee802154_rx_buffer_address" \
    --slurpfile result "$chip_root/result.json" \
    --slurpfile coverage "$chip_root/coverage.json" \
    --slurpfile requirements "$requirements" \
    '{
        schema: $schema,
        chip: $chip,
        requirement: {
            rom_file: $rom_file,
            rom_sha256: $rom_sha256,
            real_rom_image_required: true,
            vendor_headers_allowed: false,
            vendor_runtime_allowed: false,
            vendor_radio_libraries_allowed: false,
            symbol_dispatch_allowed: false,
            native_dma_read_required: true,
            native_dma_write_required: true,
            native_rf_reception_required: true,
            native_rf_submission_required: true,
            native_ble_submission_required: true,
            native_ble_reception_required: true,
	            native_ieee802154_submission_required: ($chip == "esp32c6"),
	            native_ieee802154_reception_required: ($chip == "esp32c6"),
	            native_ieee802154_multipan_filter_required: ($chip == "esp32c6"),
	            native_ieee802154_ack_success_required: ($chip == "esp32c6"),
	            native_ieee802154_no_ack_timeout_required: ($chip == "esp32c6"),
	            native_ieee802154_auto_ack_rx_required: ($chip == "esp32c6"),
	            native_ieee802154_tx_security_failure_required: ($chip == "esp32c6"),
	            native_ieee802154_cca_tx_required: ($chip == "esp32c6"),
            deterministic_replay_required: true,
            required_regions: $requirements[0].chips[$chip].required_regions,
            families_exercised: $requirements[0].chips[$chip].families_exercised
        },
        observed: {
            stop: $result[0].reason,
            exit_code: $result[0].exit_code,
            instructions: $result[0].stats.instructions,
            unique_execute_addresses: $coverage[0].unique_addresses,
            coverage_digest: $coverage[0].digest,
            firmware_elf_sha256: $elf_sha256,
            native_descriptor_address: $descriptor_address,
            native_frame_data_address: $frame_data_address,
            native_rx_descriptor_address: $rx_descriptor_address,
            native_rx_buffer_address: $rx_buffer_address,
            native_dma_read: true,
            native_dma_write: true,
            native_rf_submission: true,
            native_ble_submission: true,
            native_ble_reception: true,
	            native_ieee802154_submission: ($chip == "esp32c6"),
	            native_ieee802154_reception: ($chip == "esp32c6"),
	            native_ieee802154_multipan_filter: ($chip == "esp32c6"),
	            native_ieee802154_ack_success: ($chip == "esp32c6"),
	            native_ieee802154_no_ack_timeout: ($chip == "esp32c6"),
	            native_ieee802154_auto_ack_rx: ($chip == "esp32c6"),
	            native_ieee802154_tx_security_failure: ($chip == "esp32c6"),
	            native_ieee802154_cca_tx: ($chip == "esp32c6"),
            native_ieee802154_rx_buffer_address: $ieee802154_rx_buffer_address,
            native_ble_rx_descriptor_address: $ble_rx_descriptor_address,
            native_ble_rx_payload_address: $ble_rx_payload_address,
            native_rf_reception: true,
            deterministic_replay: true,
            radio_replay_sha256: $radio_replay_sha256
        }
    }' >"$chip_root/summary.json"

echo "$chip custom-stack radio qualification passed: $chip_root/summary.json"
