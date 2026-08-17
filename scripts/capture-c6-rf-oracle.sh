#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

requirements=qualification/radio/c6-rf-oracle-requirements.json
artifact_root=${REMU_C6_RF_ORACLE_ROOT:-.remu/qualification/c6-rf-oracle}
rom_root=$artifact_root/roms
build_root=$artifact_root/build
idf_image=$(jq -r '.esp_idf_container' "$requirements")
firmware_project=$(jq -r '.firmware_project' "$requirements")
firmware_elf=$(jq -r '.firmware_elf' "$requirements")
rom_file=$(jq -r '.rom_file' "$requirements")
expected_rom_sha=$(jq -r '.rom_sha256' "$requirements")
minimum_instructions=$(jq -r '.minimum_instructions' "$requirements")

rm -rf "$artifact_root"
mkdir -p "$artifact_root"
artifact_absolute=$(CDPATH= cd -- "$artifact_root" && pwd)

REMU_ESP_ROM_DIR="$rom_root" scripts/fetch-esp-rom-elfs.sh >"$artifact_root/rom-fetch.log"
actual_rom_sha=$(sha256sum "$rom_root/$rom_file" | cut -d ' ' -f 1)
if [ "$actual_rom_sha" != "$expected_rom_sha" ]
then
    echo "ESP32-C6 ROM hash mismatch: expected $expected_rom_sha, got $actual_rom_sha" >&2
    exit 1
fi

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --env HOME=/tmp \
    --volume "$repo_root:/workspace:ro" \
    --volume "$artifact_absolute:/out" \
    "$idf_image" \
    bash -lc "IDF_TARGET=esp32c6 SDKCONFIG_DEFAULTS=/workspace/qualification/radio/sdkconfig.rf-oracle.defaults idf.py -C /workspace/$firmware_project -B /out/build -D SDKCONFIG=/out/sdkconfig build && cd /out/build && python -m esptool --chip esp32c6 merge-bin -o /out/esp32c6-rf-oracle-flash.bin @flash_args" \
    >"$artifact_root/idf-build.log" 2>&1

if [ -z "${REMU_BIN:-}" ]
then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1} cargo build -q --release -p remu-cli --locked
    remu=target/release/remu
else
    remu=$REMU_BIN
fi

run_oracle()
{
    suffix=$1
    shift
    set -- \
        run \
        --target esp32c6 \
        --elf "$build_root/$firmware_elf" \
        --boot-rom "$rom_root/$rom_file" \
        --esp-app-image "$artifact_root/esp32c6-rf-oracle-flash.bin" \
        --max-instructions "$minimum_instructions" \
        --radio-replay "$artifact_root/radio-replay$suffix.json" \
        --result "$artifact_root/result$suffix.json" \
        --bus-log "$artifact_root/oracle-bus$suffix.json" \
        --interrupt-log "$artifact_root/interrupts$suffix.json" \
        "$@"
    for region in $(jq -r '.trace_regions[]' "$requirements")
    do
        set -- "$@" --bus-log-region "$region"
    done
    "$remu" "$@"
    python3 scripts/analyze-c6-rf-oracle.py \
        --requirements "$requirements" \
        --bus "$artifact_root/oracle-bus$suffix.json" \
        --replay "$artifact_root/radio-replay$suffix.json" \
        --output "$artifact_root/analysis$suffix.json"
}

run_oracle "" --coverage "$artifact_root/coverage.json"
run_oracle -repeat --replay "$artifact_root/result.json"

jq -e --argjson instructions "$minimum_instructions" \
    '.reason == "InstructionLimit" and .stats.instructions == $instructions' \
    "$artifact_root/result.json" >/dev/null
cmp "$artifact_root/result.json" "$artifact_root/result-repeat.json"
cmp "$artifact_root/radio-replay.json" "$artifact_root/radio-replay-repeat.json"
cmp "$artifact_root/oracle-bus.json" "$artifact_root/oracle-bus-repeat.json"
cmp "$artifact_root/interrupts.json" "$artifact_root/interrupts-repeat.json"
cmp "$artifact_root/analysis.json" "$artifact_root/analysis-repeat.json"

maximum_trace_bytes=$((64 * 1024 * 1024))
actual_trace_bytes=$(wc -c <"$artifact_root/oracle-bus.json")
if [ "$actual_trace_bytes" -gt "$maximum_trace_bytes" ]
then
    echo "C6 RF oracle trace is not bounded below $maximum_trace_bytes bytes" >&2
    exit 1
fi

elf_sha=$(sha256sum "$build_root/$firmware_elf" | cut -d ' ' -f 1)
flash_sha=$(sha256sum "$artifact_root/esp32c6-rf-oracle-flash.bin" | cut -d ' ' -f 1)
analysis_sha=$(sha256sum "$artifact_root/analysis.json" | cut -d ' ' -f 1)
jq -n \
    --arg schema remu.c6-rf-oracle-capture.v1 \
    --arg firmware_elf_sha256 "$elf_sha" \
    --arg firmware_flash_sha256 "$flash_sha" \
    --arg analysis_sha256 "$analysis_sha" \
    --argjson trace_bytes "$actual_trace_bytes" \
    --slurpfile analysis "$artifact_root/analysis.json" '
    {
        schema: $schema,
        firmware_elf_sha256: $firmware_elf_sha256,
        firmware_flash_sha256: $firmware_flash_sha256,
        analysis_sha256: $analysis_sha256,
        trace_bytes: $trace_bytes,
        deterministic: true,
        recovered_candidates: $analysis[0].recovered_candidates,
        current_airtime: [
            $analysis[0].stages[] |
            {
                name,
                center_khz: .tagged_submission.center_khz,
                power_dbm: .tagged_submission.power_dbm
            }
        ]
    }
' >"$artifact_root/summary.json"

echo "ESP32-C6 RF oracle capture passed: $artifact_root/summary.json"
