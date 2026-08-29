#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. scripts/lib/toolchain-images.sh

root=${1:-.remu/portfolio-smoke}
output=${2:-qualification/xtensa-cpu.json}
image=$(resolve_toolchain_image sha256:e0c54aeaae63f842234ec88f7b5a61b69bfa4d9005ba7490df47328e0dc9892f remu/xtensa-esp-gcc:local)
proofs=$root/xtensa-cpu-proofs.jsonl
unit_log=$root/xtensa-cpu-unit-tests.txt
mkdir -p "$(dirname -- "$output")"
: > "$proofs"

cargo test -q -p remu-cpu-xtensa > "$unit_log"
cargo test -q -p remu-machines direct_load_starts_with_appcpu_reset_and_parked >> "$unit_log"
cargo test -q -p remu-machines direct_elf_load_leaves_the_bss_tail_poisoned >> "$unit_log"
cargo test -q -p remu-machines verified_xip_requires_the_rom_instruction_cache_configuration >> "$unit_log"
cargo test -q -p remu-machines verified_handoff_requires_entry_and_rotates_callx8_window >> "$unit_log"
cargo test -q -p remu-machines esp32s3_strict_flash_boot_reports_rom_second_stage_and_handoff >> "$unit_log"

add_proof()
{
    optimization=$1
    case "$optimization" in
        o0) optimization_flag=-O0 ;;
        o2) optimization_flag=-O2 ;;
        os) optimization_flag=-Os ;;
    esac
    name=esp32s3-xtensa-$optimization
    build=$root/$name-build.json
    elf=$root/$name/smoke.elf
    result_a=$root/$name-run.json
    result_b=$root/$name-repeat-run.json
    disassembly=$root/$name-disassembly.txt

    jq -e '.reason == "Halted" and .exit_code == 0' "$result_a" >/dev/null
    jq -e '.reason == "Halted" and .exit_code == 0' "$result_b" >/dev/null
    cmp "$result_a" "$result_b"
    docker run --rm \
        -v "$project_dir:/workspace:ro" \
        -w /tmp \
        "$image" \
        xtensa-esp32s3-elf-objdump -d -h "/workspace/$elf" > "$disassembly"

    grep -Eq '[[:space:]]entry[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]call(4|8|12)[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]retw(\.n)?([[:space:]]|$)' "$disassembly"
    grep -Eq '[[:space:]]s32c1i[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]wsr\.scompare1[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]](mul|madd)\.s[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]oeq\.s[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]wsr\.vecbase[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]wsr\.intset[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]wsr\.intclear[[:space:]]' "$disassembly"
    grep -Eq '[[:space:]]rfe([[:space:]]|$)' "$disassembly"
    grep -Eq '^  [0-9]+ \.text +[0-9a-f]+ +4037' "$disassembly"
    grep -Eq '^  [0-9]+ \.level1_vector +[0-9a-f]+ +40378340' "$disassembly"
    grep -Eq '^  [0-9]+ \.irom +[0-9a-f]+ +42000000' "$disassembly"
    grep -Eq '^  [0-9]+ \.drom +[0-9a-f]+ +3c000000' "$disassembly"

    build_sha=$(jq -S -c 'del(.stdout, .stderr)' "$build" \
        | sha256sum | cut -d ' ' -f 1)
    elf_sha=$(sha256sum "$elf" | cut -d ' ' -f 1)
    result_sha=$(sha256sum "$result_a" | cut -d ' ' -f 1)
    disassembly_sha=$(sha256sum "$disassembly" | cut -d ' ' -f 1)
    jq -c \
        --arg id "xtensa-$optimization" \
        --arg optimization "$optimization_flag" \
        --arg build_artifact "$build" \
        --arg build_provenance_sha256 "$build_sha" \
        --arg elf "$elf" \
        --arg elf_sha256 "$elf_sha" \
        --arg result_artifact "$result_a" \
        --arg repeat_result_artifact "$result_b" \
        --arg result_sha256 "$result_sha" \
        --arg disassembly_artifact "$disassembly" \
        --arg disassembly_sha256 "$disassembly_sha" \
        '{
          id: $id,
          target: "esp32s3",
          cpu: "xtensa-lx7",
          optimization: $optimization,
          build_artifact: $build_artifact,
          build_provenance_sha256: $build_provenance_sha256,
          elf: $elf,
          elf_sha256: $elf_sha256,
          result_artifact: $result_artifact,
          repeat_result_artifact: $repeat_result_artifact,
          result_sha256: $result_sha256,
          disassembly_artifact: $disassembly_artifact,
          disassembly_sha256: $disassembly_sha256,
          reason,
          exit_code,
          stats,
          deterministic_repeat: true,
          features: [
            "windowed ABI and register windows",
            "S32C1I atomic compare/store",
            "single-precision FPU",
            "level-one exception entry and RFE",
            "IRAM/DRAM/IROM/DROM direct ELF views"
          ],
          result: "pass"
        }' "$result_a" >> "$proofs"
}

add_proof o0
add_proof o2
add_proof os

image_id=$(docker image inspect --format '{{.Id}}' "$image")
bss_missing=$root/esp32s3-bss-missing-run.json
bss_cleared=$root/esp32s3-bss-cleared-run.json
jq -e '.reason == "Halted" and .exit_code == 70' "$bss_missing" >/dev/null
jq -e '.reason == "Halted" and .exit_code == 0' "$bss_cleared" >/dev/null
bss_missing_sha=$(sha256sum "$bss_missing" | cut -d ' ' -f 1)
bss_cleared_sha=$(sha256sum "$bss_cleared" | cut -d ' ' -f 1)
source_sha=$(sha256sum \
    crates/remu-cpu-xtensa/src/lib.rs \
    crates/remu-cpu-xtensa/src/core/execution.rs \
    crates/remu-cpu-xtensa/src/core/tests.rs \
    crates/remu-machines/src/xtensa.rs \
    crates/remu-machines/src/xtensa/boot.rs \
    crates/remu-machines/src/xtensa/machine_runtime.rs \
    crates/remu-machines/src/xtensa/tests_image.rs \
    corpus/smoke/xtensa-bss/link.ld \
    corpus/smoke/xtensa-bss/main.c \
    corpus/smoke/xtensa-qualification/exception.S \
    corpus/smoke/xtensa-qualification/link.ld \
    corpus/smoke/xtensa-qualification/main.c \
    scripts/docker-smoke.sh \
    scripts/generate-xtensa-cpu-qualification.sh | sha256sum | cut -d ' ' -f 1)
unit_sha=$(sha256sum "$unit_log" | cut -d ' ' -f 1)

jq -n \
    --arg schema remu.xtensa-cpu.v1 \
    --arg source_sha256 "$source_sha" \
    --arg unit_test_artifact "$unit_log" \
    --arg unit_test_sha256 "$unit_sha" \
    --arg image "$image" \
    --arg image_id "$image_id" \
    --arg bss_missing "$bss_missing" \
    --arg bss_missing_sha256 "$bss_missing_sha" \
    --arg bss_cleared "$bss_cleared" \
    --arg bss_cleared_sha256 "$bss_cleared_sha" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      primary_sources: [
        "https://documentation.espressif.com/esp32-s3_technical_reference_manual_en.pdf",
        "https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/tools/idf-tools.html",
        "https://github.com/espressif/crosstool-NG"
      ],
      toolchain: {
        name: "xtensa-esp32s3-elf-gcc",
        container_reference: $image,
        container_image_id: $image_id,
        default_abi: "windowed"
      },
      implemented: [
        "16-bit density and 24-bit compiler instruction decoding",
        "integer, branch, zero-overhead loop, multiply and divide operations",
        "windowed calls, ENTRY and RETW register-window behavior",
        "level-one interrupt entry, INTLEVEL state and RFE",
        "S32C1I atomic compare/store and SCOMPARE1",
        "single-precision LX7 FPU compiler operations",
        "ESP32-S3 IRAM, DRAM, IROM and DROM direct ELF mappings",
        "word-aligned CALLX0/4/8/12 indirect targets",
        "nonzero power-on RAM with file-backed-only ELF loading",
        "cache-gated verified IROM fetch and CALLX8/ENTRY handoff",
        "validated functional ROM/second-stage native flash boot plan",
        "CPU0 direct execution with CPU1 reset and parked"
      ],
      bss_startup: {
        missing_clear: {
          artifact: $bss_missing,
          sha256: $bss_missing_sha256,
          expected_exit_code: 70,
          result: "pass"
        },
        explicit_clear: {
          artifact: $bss_cleared,
          sha256: $bss_cleared_sha256,
          expected_exit_code: 0,
          result: "pass"
        }
      },
      unit_tests: {
        artifact: $unit_test_artifact,
        sha256: $unit_test_sha256
      },
      proofs: $proofs,
      result: "pass"
    }' > "$output"

echo "Xtensa CPU qualification passed; artifact: $output"
