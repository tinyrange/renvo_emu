#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

root=${1:-.renvo/portfolio-smoke}
output=${2:-qualification/riscv-cpu.json}
proofs=$root/riscv-cpu-proofs.jsonl
unit_log=$root/riscv-cpu-unit-tests.txt
mkdir -p "$(dirname -- "$output")"
: > "$proofs"

cargo test -q -p renvo-cpu-riscv qingke_ > "$unit_log"
cargo test -q -p renvo-cpu-riscv esp32c6_ >> "$unit_log"
cargo test -q -p renvo-cpu-riscv hazard3_ >> "$unit_log"

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

add_proof ch32v003-xw ch32v003 qingke-v2a xw-eight-operations \
    "$root/wch-xw-build.json" "$root/wch-xw/smoke.elf" \
    "$root/ch32v003-xw-run.json"
add_proof ch32v006-xw ch32v006 qingke-v2c xw-eight-operations \
    "$root/wch-xw-build.json" "$root/wch-xw/smoke.elf" \
    "$root/ch32v006-xw-run.json"
add_proof ch32v006-zmmul ch32v006 qingke-v2c zmmul \
    "$root/wch-zmmul-build.json" "$root/wch-zmmul/smoke.elf" \
    "$root/ch32v006-zmmul-run.json"
add_proof esp32c6-privilege esp32c6 esp-rv32imac-hp user-traps-and-pmp-visibility \
    "$root/esp32c6-privilege-build.json" "$root/esp32c6-privilege/smoke.elf" \
    "$root/esp32c6-privilege-run.json"
add_proof rp2350-hazard3-extensions rp2350 hazard3 documented-compiler-march \
    "$root/hazard3-build.json" "$root/hazard3/smoke.elf" \
    "$root/hazard3-extensions-run.json"

grep -qx 'rv32imac_zicsr_zifencei_zba_zbb_zbs_zbkb_zcb_zcmp' \
    "$root/hazard3/full-march.txt"

source_sha=$(sha256sum \
    crates/renvo-cpu-riscv/src/lib.rs \
    corpus/smoke/wch-xw/link.ld \
    corpus/smoke/wch-xw/start.S \
    corpus/smoke/wch-xw/zmmul.S \
    corpus/smoke/esp32c6/privilege.S \
    corpus/smoke/hazard3/build.sh \
    corpus/smoke/hazard3/compiler-cases.c \
    corpus/smoke/hazard3/extensions.S \
    corpus/smoke/hazard3/link.ld \
    corpus/smoke/hazard3/start.S \
    scripts/docker-smoke.sh \
    scripts/generate-riscv-cpu-qualification.sh | sha256sum | cut -d ' ' -f 1)
unit_sha=$(sha256sum "$unit_log" | cut -d ' ' -f 1)

jq -n \
    --arg schema renvo.riscv-cpu.v1 \
    --arg source_sha256 "$source_sha" \
    --arg unit_test_artifact "$unit_log" \
    --arg unit_test_sha256 "$unit_sha" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      primary_sources: [
        "https://www.wch-ic.com/downloads/QingKeV2_Processor_Manual_PDF.html",
        "https://www.espressif.com/sites/default/files/documentation/esp32-c6_technical_reference_manual_en.pdf",
        "https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf",
        "https://wren.wtf/hazard3/doc/"
      ],
      implemented: {
        xw: ["c.lbu", "c.lhu", "c.sb", "c.sh", "c.lbusp", "c.lhusp", "c.sbsp", "c.shsp"],
        v2c: ["Zmmul"],
        esp32c6: ["machine/user transition", "user ECALL", "privileged CSR trap", "PMP CSR visibility"],
        hazard3: ["RV32IMAC", "Zicsr", "Zifencei", "Zba", "Zbb", "Zbs", "Zbkb", "Zcb", "Zcmp"]
      },
      negative_tests: {
        artifact: $unit_test_artifact,
        sha256: $unit_test_sha256,
        assertions: [
          "XW is illegal outside QingKe profiles",
          "V2A rejects Zmmul",
          "V2C rejects M-extension divide",
          "ESP32-C6 user ECALL enters machine mode",
          "ESP32-C6 user access to a machine CSR traps as illegal",
          "Hazard3 Zcb compact byte/halfword memory operations execute",
          "Hazard3 Zcmp push/popret preserves the architectural register list"
        ]
      },
      proofs: $proofs,
      result: "pass"
    }' > "$output"

echo "RISC-V CPU qualification passed; artifact: $output"
