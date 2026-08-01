#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

root=${1:-.remu/portfolio-smoke}
output=${2:-qualification/arm-cpu.json}
proofs=$root/arm-cpu-proofs.jsonl
unit_log=$root/arm-cpu-unit-tests.txt
mkdir -p "$(dirname -- "$output")"
: > "$proofs"

cargo test -q -p remu-cpu-arm > "$unit_log"
cargo test -q -p remu-devices arm_ppb_ >> "$unit_log"
cargo test -q -p remu-devices rp_sio_echoes_bootrom_launch >> "$unit_log"

add_proof()
{
    id=$1
    target=$2
    cpu=$3
    feature=$4
    build=$5
    elf=$6
    result=$7

    jq -e '.reason == "Halted" and .exit_code == 0' "$result" >/dev/null
    build_sha=$(jq -S -c 'del(.stdout, .stderr)' "$build" \
        | sha256sum | cut -d ' ' -f 1)
    elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
    result_sha=$(sha256sum "$result" | cut -d ' ' -f 1)
    jq -c \
        --arg id "$id" \
        --arg target "$target" \
        --arg cpu "$cpu" \
        --arg feature "$feature" \
        --arg build_artifact "$build" \
        --arg build_provenance_sha256 "$build_sha" \
        --arg elf "$elf" \
        --arg elf_sha256 "$elf_sha" \
        --arg result_artifact "$result" \
        --arg result_sha256 "$result_sha" \
        '{
          id: $id,
          target: $target,
          cpu: $cpu,
          feature: $feature,
          build_artifact: $build_artifact,
          build_provenance_sha256: $build_provenance_sha256,
          elf: $elf,
          elf_sha256: $elf_sha256,
          result_artifact: $result_artifact,
          result_sha256: $result_sha256,
          reason,
          exit_code,
          stats,
          trace_digest,
          result: "pass"
        }' "$result" >> "$proofs"
}

add_proof rp2040-exceptions rp2040 cortex-m0plus systick-and-multibank-nvic \
    "$root/rp2040-arm-exceptions-build.json" \
    "$root/rp2040-arm-exceptions/smoke.elf" \
    "$root/rp2040-arm-exceptions-run.json"
add_proof rp2350-exceptions rp2350 cortex-m33 systick-and-multibank-nvic \
    "$root/rp2350-arm-exceptions-build.json" \
    "$root/rp2350-arm-exceptions/smoke.elf" \
    "$root/rp2350-arm-exceptions-run.json"
add_proof rp2350-hardfloat-dsp rp2350 cortex-m33 fpv5-sp-and-dsp \
    "$root/rp2350-arm-hardfloat-build.json" \
    "$root/rp2350-arm-hardfloat/smoke.elf" \
    "$root/rp2350-arm-hardfloat-run.json"

jq -e '.argv | index("-mfpu=fpv5-sp-d16") and index("-mfloat-abi=hard")' \
    "$root/rp2350-arm-hardfloat-build.json" >/dev/null
jq -e '[.proofs[] | select(.target == "rp2350" and
    (.cpu == "cortex-m33" or .cpu == "hazard3-rv32imac") and .result == "pass")] | length == 6' \
    qualification/rust-abi.json >/dev/null

source_sha=$(sha256sum \
    crates/remu-cpu-arm/src/lib.rs \
    crates/remu-devices/src/lib.rs \
    crates/remu-machines/src/arm.rs \
    corpus/smoke/arm-qualification/exceptions.S \
    corpus/smoke/arm-qualification/hardfloat.c \
    corpus/smoke/arm-qualification/link.ld \
    corpus/smoke/arm-qualification/start.S \
    scripts/docker-smoke.sh \
    scripts/generate-arm-cpu-qualification.sh | sha256sum | cut -d ' ' -f 1)
unit_sha=$(sha256sum "$unit_log" | cut -d ' ' -f 1)
cross_profile_sha=$(sha256sum qualification/rust-abi.json | cut -d ' ' -f 1)

jq -n \
    --arg schema remu.arm-cpu.v1 \
    --arg source_sha256 "$source_sha" \
    --arg unit_test_artifact "$unit_log" \
    --arg unit_test_sha256 "$unit_sha" \
    --arg cross_profile_sha256 "$cross_profile_sha" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      primary_sources: [
        "https://developer.arm.com/documentation/100235/0003/",
        "https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf",
        "https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf"
      ],
      implemented: {
        cortex_m0plus: ["Armv6-M compiler baseline", "exception stacking/return", "SysTick", "NVIC banks 0-7"],
        cortex_m33: ["Armv8-M Mainline compiler baseline", "DSP multiply-accumulate", "FPv5 single precision", "exception stacking/return", "SysTick", "NVIC banks 0-7"],
        baseline_policy: ["non-secure execution", "TrustZone unsupported"]
      },
      unit_tests: {
        artifact: $unit_test_artifact,
        sha256: $unit_test_sha256
      },
      cross_profile_computation: {
        artifact: "qualification/rust-abi.json",
        sha256: $cross_profile_sha256,
        assertion: "the same Rust ABI computation passes Cortex-M33 and Hazard3 at O0/O2/Os"
      },
      proofs: $proofs,
      result: "pass"
    }' > "$output"

echo "Arm CPU qualification passed; artifact: $output"
