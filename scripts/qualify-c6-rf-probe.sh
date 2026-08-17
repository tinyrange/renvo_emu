#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

requirements=qualification/radio/c6-rf-probe/requirements.json
source_root=qualification/radio/c6-rf-probe
artifact_root=${REMU_C6_RF_PROBE_ROOT:-.remu/qualification/c6-rf-probe}
out=$artifact_root/out
base=$(jq -r '.base_flash.path' "$requirements")
base_sha=$(jq -r '.base_flash.sha256' "$requirements")
offset=$(jq -r '.base_flash.application_offset' "$requirements")
max_instructions=$(jq -r '.max_instructions' "$requirements")
package_image=$(resolve_toolchain_image sha256:5aa633e02afc7f2657a5ddc76bdd1ba0f720545e8d6b8690eeca045c29496e09 remu/nanoc6-esptool:5.3.0)
rom_root=${REMU_ESP_ROM_DIR:-.remu/qualification/esp-rom-elfs/20260528}
rom=$rom_root/esp32c6_rev0_rom.elf

test -s "$base"
test "$(sha256sum "$base" | cut -d ' ' -f 1)" = "$base_sha"
if [ ! -s "$rom" ]; then
    REMU_ESP_ROM_DIR=$rom_root scripts/fetch-esp-rom-elfs.sh >/dev/null
fi
docker image inspect "$package_image" >/dev/null

rm -rf "$artifact_root"
mkdir -p "$out"
if [ -z "${REMU_BIN:-}" ]; then
    cargo build -q -p remu-cli --locked
    remu=target/debug/remu
else
    remu=$REMU_BIN
fi

"$remu" corpus build \
    --toolchain toolchains/riscv32-esp-gcc-esp32c6.toml \
    --source "$source_root" --output "$out" --target esp32c6 \
    --artifact "$artifact_root/build.json" -- \
    -O2 -Wall -Wextra -Werror start.S main.c station.c \
    hal/c6/dma.c hal/c6/rf.c hal/c6/phy.c hal/c6/mac.c hal/c6/uart.c \
    -Wl,-T,link-esp32c6.ld,-Map,/workspace/out/c6-rf-probe.map \
    -o /workspace/out/c6-rf-probe.elf

if [ -n "$(nm -u "$out/c6-rf-probe.elf")" ]; then
    echo "c6-rf-probe has undefined symbols" >&2
    nm -u "$out/c6-rf-probe.elf" >&2
    exit 1
fi
if nm "$out/c6-rf-probe.elf" | rg -i '[[:space:]](esp_wifi_|esp_phy_|net80211|pp_attach|wpa_supplicant)' >/dev/null; then
    echo "c6-rf-probe links a prohibited vendor radio symbol" >&2
    exit 1
fi

out_absolute=$(CDPATH= cd -- "$out" && pwd)
docker run --rm --network=none --pull=never \
    --entrypoint /opt/esptool-5.3/bin/esptool --volume "$out_absolute:/work" \
    "$package_image" --chip esp32c6 elf2image \
    --output /work/c6-rf-probe-app.bin /work/c6-rf-probe.elf >/dev/null
python3 scripts/package-native.py overlay --base "$base" \
    --application "$out/c6-rf-probe-app.bin" \
    --output "$artifact_root/c6-rf-probe-flash.bin" --offset "$offset"

run_case()
{
    name=$1
    input=${2:-}
    set -- "$remu" firmware boot --target esp32c6 \
        --image "$artifact_root/c6-rf-probe-flash.bin" --boot-rom "$rom" \
        --max-instructions "$max_instructions" \
        --radio-replay "$artifact_root/$name-replay.json" \
        --result "$artifact_root/$name-result.json"
    if [ -n "$input" ]; then set -- "$@" --radio-input "$input"; fi
    "$@"
}

run_case run-a
run_case run-b
cmp "$artifact_root/run-a-result.json" "$artifact_root/run-b-result.json"
cmp "$artifact_root/run-a-replay.json" "$artifact_root/run-b-replay.json"
run_case on-channel "$source_root/radio-on-channel.json"
run_case wrong-channel "$source_root/radio-wrong-channel.json"

"$remu" firmware boot --target esp32c6 \
    --image "$artifact_root/c6-rf-probe-flash.bin" --boot-rom "$rom" \
    --max-instructions "$max_instructions" \
    --radio-script "$source_root/open-ap-peer.star" \
    --radio-replay "$artifact_root/open-station-replay.json" \
    --result "$artifact_root/open-station-result.json"
"$remu" firmware boot --target esp32c6 \
    --image "$artifact_root/c6-rf-probe-flash.bin" --boot-rom "$rom" \
    --max-instructions "$max_instructions" \
    --radio-script "$source_root/open-ap-peer.star" \
    --radio-replay "$artifact_root/open-station-repeat-replay.json" \
    --result "$artifact_root/open-station-repeat-result.json"
cmp "$artifact_root/open-station-result.json" "$artifact_root/open-station-repeat-result.json"
cmp "$artifact_root/open-station-replay.json" "$artifact_root/open-station-repeat-replay.json"

uart=$(jq -r '.uart | implode' "$artifact_root/run-a-result.json")
jq -r '.required_checkpoints[]' "$requirements" | while IFS= read -r checkpoint; do
    if ! printf '%s' "$uart" | rg -F "event=$checkpoint result=0" >/dev/null; then
        echo "missing successful checkpoint $checkpoint" >&2
        exit 1
    fi
done

jq -c '.required_airtime[]' "$requirements" | while IFS= read -r expected; do
    ssid=$(printf '%s' "$expected" | jq -r '.ssid')
    center=$(printf '%s' "$expected" | jq -r '.center_khz')
    power=$(printf '%s' "$expected" | jq -r '.power_dbm')
    jq -e --arg ssid "$ssid" --argjson center "$center" --argjson power "$power" '
        any(.events[];
            .event == "submitted" and
            .request.frame.origin == "emulated" and
            .request.frame.spectrum.center_khz == $center and
            .request.frame.spectrum.bandwidth_khz == 20000 and
            .request.power_dbm == $power and
            (.request.frame.bytes[26:] | implode) == $ssid)
    ' "$artifact_root/run-a-replay.json" >/dev/null
done

jq -e 'any(.events[]; .event == "reception" and .receiver == 1 and .outcome.kind == "delivered")' \
    "$artifact_root/on-channel-replay.json" >/dev/null
jq -e 'all(.events[]; .event != "reception" or .receiver != 1)' \
    "$artifact_root/wrong-channel-replay.json" >/dev/null
jq -e '.uart | implode | contains("event=RX result=0")' \
    "$artifact_root/on-channel-result.json" >/dev/null
jq -e '.uart | implode | contains("event=RX result=0") | not' \
    "$artifact_root/wrong-channel-result.json" >/dev/null
jq -r '.open_station_checkpoints[]' "$requirements" | while IFS= read -r checkpoint; do
    if ! jq -e --arg checkpoint "$checkpoint" \
        '.uart | implode | contains("event=" + $checkpoint + " result=0")' \
        "$artifact_root/open-station-result.json" >/dev/null; then
        echo "open station omitted checkpoint $checkpoint" >&2
        exit 1
    fi
done
jq -e '
    any(.events[]; .event == "submitted" and
        .request.frame.origin == "emulated" and
        .request.frame.bytes[0:2] == [176, 0]) and
    any(.events[]; .event == "submitted" and
        .request.frame.origin == "emulated" and
        .request.frame.bytes[0:2] == [0, 0]) and
    any(.events[]; .event == "submitted" and
        .request.frame.origin == "emulated" and
        .request.frame.bytes[0:2] == [8, 1] and
        .request.frame.bytes[24:36] == [170,170,3,0,0,0,136,181,80,73,78,71]) and
    any(.events[]; .event == "submitted" and
        .request.frame.origin == "host-injection" and
        .request.frame.bytes[0:2] == [8, 2] and
        .request.frame.bytes[24:36] == [170,170,3,0,0,0,136,181,80,79,78,71])
' "$artifact_root/open-station-replay.json" >/dev/null

elf_sha=$(sha256sum "$out/c6-rf-probe.elf" | cut -d ' ' -f 1)
app_sha=$(sha256sum "$out/c6-rf-probe-app.bin" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$artifact_root/c6-rf-probe-flash.bin" | cut -d ' ' -f 1)
jq -n --arg elf_sha "$elf_sha" --arg app_sha "$app_sha" \
    --arg flash_sha "$flash_sha" --arg base_sha "$base_sha" \
    --arg rom_sha "$(sha256sum "$rom" | cut -d ' ' -f 1)" '{
        schema: "remu.c6-rf-probe-summary.v1",
        target: "esp32c6",
        independent_link_audit: "pass",
        deterministic_repeat: "pass",
        renvo_genuine_rom: "pass",
        channel_power_airtime: "pass",
        receive_on_channel: "pass",
        reject_wrong_channel: "pass",
        open_system_station: "pass",
        bidirectional_l2: "pass",
        physical_hardware: "not-run",
        hashes: {elf: $elf_sha, application: $app_sha, exact_flash: $flash_sha,
                 base_flash: $base_sha, mask_rom: $rom_sha}
    }' >"$artifact_root/summary.json"
jq . "$artifact_root/summary.json"
