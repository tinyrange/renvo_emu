#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
root=${REDUCTION_ARTIFACT_ROOT:-.remu/qualification/reduction}
summary=qualification/reduction.json
mkdir -p "$root"

reduce_family()
{
    family=$1
    target=$2
    toolchain=$3
    source=$4
    shift 4
    output=$root/$family
    artifact=$root/$family.json
    "$remu" corpus reduce \
        --target "$target" \
        --toolchain "$toolchain" \
        --source "$source" \
        --output "$output" \
        --seed-expected 0 \
        --source-item '#define REMU_NOISE_A 17' \
        --source-item '#define REMU_SOURCE_TRIGGER 1' \
        --source-item '#define REMU_NOISE_B 29' \
        --flag-item=-fno-common \
        --flag-item=-DREMU_FLAG_TRIGGER=1 \
        --flag-item=-fno-strict-aliasing \
        --input-item 0 \
        --input-item 7 \
        --input-item 0 \
        --artifact "$artifact" \
        -- "$@"
    jq -e '
      .result == "pass" and
      .final_reproducible == true and
      .seeded_expected == 0 and
      (.reduction.minimized.source == ["#define REMU_SOURCE_TRIGGER 1"]) and
      (.reduction.minimized.flags == ["-DREMU_FLAG_TRIGGER=1"]) and
      (.reduction.minimized.inputs == [7]) and
      ([.evaluations[] | select(.discrepancy)] | length > 0) and
      ([.evaluations[] | select(.discrepancy == false)] | length > 0)
    ' "$artifact" >/dev/null
}

reduce_family riscv ch32v003 toolchains/riscv-gcc-rv32ec.toml \
    corpus/reduction/riscv \
    -O2 -Wl,-T,link.ld -o /workspace/out/smoke.elf main.c
reduce_family arm rp2040 toolchains/arm-gcc-cortex-m0plus.toml \
    corpus/reduction/arm \
    -O2 -Wl,-T,link.ld -o /workspace/out/smoke.elf main.c
reduce_family xtensa esp32s3 toolchains/xtensa-esp-gcc-esp32s3.toml \
    corpus/reduction/xtensa \
    -O2 -Wl,-T,link.ld -o /workspace/out/smoke.elf main.c

source_sha=$(sha256sum \
    crates/remu-corpus/src/reduce.rs \
    crates/remu-cli/src/main.rs \
    corpus/reduction/riscv/main.c corpus/reduction/riscv/link.ld \
    corpus/reduction/arm/main.c corpus/reduction/arm/link.ld \
    corpus/reduction/xtensa/main.c corpus/reduction/xtensa/link.ld \
    scripts/qualify-reduction.sh | sha256sum | cut -d ' ' -f 1)

proofs=$root/proofs.jsonl
: > "$proofs"
for family in riscv arm xtensa
do
    artifact=$root/$family.json
    artifact_sha=$(sha256sum "$artifact" | cut -d ' ' -f 1)
    jq -c \
        --arg family "$family" \
        --arg artifact "$artifact" \
        --arg artifact_sha256 "$artifact_sha" \
        '{
          family: $family,
          target,
          artifact: $artifact,
          artifact_sha256: $artifact_sha256,
          seeded_expected,
          original: .reduction.original,
          minimized: .reduction.minimized,
          evaluations: (.evaluations | length),
          final_repeat_evaluations,
          final_reproducible,
          result
        }' "$artifact" >> "$proofs"
done

jq -n \
    --arg schema remu.reduction-qualification.v1 \
    --arg source_sha256 "$source_sha" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      axes: ["source", "compiler_flags", "external_inputs"],
      policy: "seeded reference discrepancy; deterministic source/flag/input ddmin; two final compile-and-run repeats",
      proofs: $proofs,
      result: "pass"
    }' > "$summary"

echo "seeded discrepancies reduced on RISC-V, Arm, and Xtensa; artifact: $summary"
