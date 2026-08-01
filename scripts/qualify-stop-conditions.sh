#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

remu=${REMU_BIN:-target/debug/remu}
portfolio=${REMU_PORTFOLIO_ARTIFACTS:-.remu/portfolio-smoke}
root=.remu/stop-conditions
artifact=qualification/stop-conditions.json
mkdir -p "$root"

test -x "$remu"
for elf in \
    "$portfolio/wch/smoke.elf" \
    "$portfolio/rp-arm/smoke.elf" \
    "$portfolio/esp32s3/smoke.elf" \
    "$portfolio/stop-riscv-fault/smoke.elf" \
    "$portfolio/stop-arm-fault/smoke.elf" \
    "$portfolio/stop-xtensa-fault/smoke.elf"
do
    test -f "$elf"
done

rv_elf=$portfolio/wch/smoke.elf
arm_elf=$portfolio/rp-arm/smoke.elf
xtensa_elf=$portfolio/esp32s3/smoke.elf
rv_entry=$($remu inspect "$rv_elf" | jq -r '.entry')
arm_entry=$($remu inspect "$arm_elf" | jq -r '.entry - (.entry % 2)')
xtensa_entry=$($remu inspect "$xtensa_elf" | jq -r '.entry')

run()
{
    id=$1
    target=$2
    elf=$3
    shift 3
    max_instructions=100
    if test "${1:-}" = --limit
    then
        max_instructions=$2
        shift 2
    fi
    "$remu" run \
        --target "$target" \
        --elf "$elf" \
        --max-instructions "$max_instructions" \
        "$@" \
        --result "$root/$id.json"
}

run riscv-breakpoint ch32v003 "$rv_elf" --breakpoint "$rv_entry"
run arm-breakpoint rp2040 "$arm_elf" --breakpoint "$arm_entry"
run xtensa-breakpoint esp32s3 "$xtensa_elf" --breakpoint "$xtensa_entry"

run riscv-watchpoint ch32v003 "$rv_elf" --watchpoint 0xfffffff0
run arm-watchpoint rp2040 "$arm_elf" --watchpoint 0xfffffff0
run xtensa-watchpoint esp32s3 "$xtensa_elf" --watchpoint 0xfffffff0

run riscv-signal ch32v003 "$rv_elf" \
    --stop-signal board.ch32v003.gpioc.pin1=rising
run arm-signal rp2040 "$arm_elf" \
    --stop-signal board.rp2040.chip_gpio.pin25=rising
run xtensa-signal esp32s3 "$xtensa_elf" \
    --stop-signal board.esp32s3.chip_gpio.pin2=rising

run riscv-instruction-limit ch32v003 "$rv_elf" --limit 1
run arm-instruction-limit rp2040 "$arm_elf" --limit 1
run xtensa-instruction-limit esp32s3 "$xtensa_elf" --limit 1

run riscv-time-limit ch32v003 "$rv_elf" --deadline 1
run arm-time-limit rp2040 "$arm_elf" --deadline 1
run xtensa-time-limit esp32s3 "$xtensa_elf" --deadline 1

run riscv-fault ch32v003 "$portfolio/stop-riscv-fault/smoke.elf"
run arm-fault rp2040 "$portfolio/stop-arm-fault/smoke.elf"
run xtensa-fault esp32s3 "$portfolio/stop-xtensa-fault/smoke.elf"

for architecture in riscv arm xtensa
do
    jq -e '.reason == "Breakpoint" and .stats.instructions == 0' \
        "$root/$architecture-breakpoint.json" >/dev/null
    jq -e '.reason.Watchpoint.address == 4294967280 and .reason.Watchpoint.access == "Write"' \
        "$root/$architecture-watchpoint.json" >/dev/null
    jq -e '.reason.Signal | type == "string"' \
        "$root/$architecture-signal.json" >/dev/null
    jq -e '.reason == "InstructionLimit" and .stats.instructions == 1' \
        "$root/$architecture-instruction-limit.json" >/dev/null
    jq -e '.reason == "TimeLimit" and .stats.time == 1' \
        "$root/$architecture-time-limit.json" >/dev/null
    jq -e '.reason.Fault | type == "string" and length > 0' \
        "$root/$architecture-fault.json" >/dev/null
done

work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
proofs=$work/proofs.ndjson
: > "$proofs"
for condition in breakpoint fault instruction-limit signal time-limit watchpoint
do
    for architecture in arm riscv xtensa
    do
        id=$architecture-$condition
        result=$root/$id.json
        sha=$(sha256sum "$result" | cut -d ' ' -f 1)
        jq --arg id "$id" --arg artifact "$result" --arg sha256 "$sha" \
            '{id: $id, artifact: $artifact, sha256: $sha256, target, reason, stats, trace_digest, result: "pass"}' \
            "$result" >> "$proofs"
    done
done

source_sha=$(sha256sum \
    corpus/smoke/wch-gpio/fault.c \
    corpus/smoke/rp-sio/fault.c \
    corpus/smoke/xtensa/fault.c \
    crates/remu-core/src/run.rs \
    crates/remu-bus/src/lib.rs \
    crates/remu-signals/src/lib.rs \
    crates/remu-machines/src/lib.rs \
    crates/remu-machines/src/riscv.rs \
    crates/remu-machines/src/arm.rs \
    crates/remu-machines/src/xtensa.rs \
    crates/remu-cpu-arm/src/lib.rs \
    crates/remu-cpu-riscv/src/lib.rs \
    crates/remu-cpu-xtensa/src/lib.rs \
    crates/remu-cli/src/main.rs \
    scripts/qualify-stop-conditions.sh | sha256sum | cut -d ' ' -f 1)
jq -n \
    --arg schema "remu.stop-conditions.v1" \
    --arg source_sha256 "$source_sha" \
    --slurpfile proofs "$proofs" \
    '{
      schema: $schema,
      source_sha256: $source_sha256,
      cpu_families: ["riscv", "arm", "xtensa"],
      conditions: ["exit", "fault", "breakpoint", "watchpoint", "signal-edge", "virtual-time", "instruction-budget"],
      proofs: $proofs
    }' > "$artifact"

test "$(jq '.proofs | length' "$artifact")" -eq 18
echo "stop conditions passed on RISC-V, Arm, and Xtensa; artifact: $artifact"
