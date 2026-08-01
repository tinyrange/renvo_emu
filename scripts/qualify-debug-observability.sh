#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/debug-observability
artifact=qualification/debug-observability.json
rm -rf "$root"
mkdir -p "$root"

sha256()
{
    sha256sum "$1" | cut -d ' ' -f 1
}

qualify()
{
    family=$1
    target=$2
    elf=$3
    architecture=$4
    case_root="$root/$family"
    ready="$case_root/gdb-ready.json"
    session="$case_root/gdb-session.json"
    transcript="$case_root/gdb-transcript.json"
    baseline="$case_root/run.json"
    repeat="$case_root/run-repeat.json"
    coverage="$case_root/coverage.json"
    repeat_coverage="$case_root/coverage-repeat.json"
    mkdir -p "$case_root"

    "$remu" run --target "$target" --elf "$elf" --max-instructions 10000 \
        --result "$baseline" --coverage "$coverage"
    "$remu" run --target "$target" --elf "$elf" --max-instructions 10000 \
        --replay "$baseline" --result "$repeat" --coverage "$repeat_coverage"
    cmp "$baseline" "$repeat"
    cmp "$coverage" "$repeat_coverage"
    jq -e '.schema == "remu.execution-coverage.v1" and .fetch_accesses > 0 and .unique_addresses > 0 and (.digest | length == 64)' "$coverage" >/dev/null

    "$remu" gdb --target "$target" --elf "$elf" --listen 127.0.0.1:0 \
        --ready "$ready" --artifact "$session" >"$case_root/gdb.log" 2>&1 &
    server_pid=$!
    attempts=0
    while [ ! -s "$ready" ] && kill -0 "$server_pid" 2>/dev/null
    do
        attempts=$((attempts + 1))
        if [ "$attempts" -gt 200 ]; then
            kill "$server_pid" 2>/dev/null || true
            wait "$server_pid" 2>/dev/null || true
            echo "GDB server did not become ready for $family" >&2
            exit 1
        fi
        sleep 0.01
    done
    if [ ! -s "$ready" ]; then
        wait "$server_pid"
        echo "GDB server exited before becoming ready for $family" >&2
        exit 1
    fi
    address=$(jq -r '.address' "$ready")
    entry=$("$remu" inspect "$elf" | jq -r '.entry')
    python3 qualification/gdb_client.py "$address" "$entry" "$architecture" "$transcript"
    wait "$server_pid"

    jq -e '.schema == "remu.gdb-session.v1" and .result == "pass" and .report.detached == true and .report.packets >= 11 and .report.register_reads >= 2 and .report.memory_reads >= 1 and .report.breakpoint_operations >= 2 and .report.steps >= 1 and .report.continues >= 1' "$session" >/dev/null
    jq -e '.schema == "remu.gdb-client-transcript.v1" and .result == "pass" and (.packets | length) >= 11' "$transcript" >/dev/null

    jq -n \
        --arg family "$family" \
        --arg target "$target" \
        --arg architecture "$architecture" \
        --arg elf "$elf" \
        --arg elf_sha256 "$(sha256 "$elf")" \
        --arg run "$baseline" \
        --arg run_sha256 "$(sha256 "$baseline")" \
        --arg coverage "$coverage" \
        --arg coverage_sha256 "$(sha256 "$coverage")" \
        --arg session "$session" \
        --arg session_sha256 "$(sha256 "$session")" \
        --arg transcript "$transcript" \
        --arg transcript_sha256 "$(sha256 "$transcript")" \
        --slurpfile coverage_data "$coverage" \
        --slurpfile session_data "$session" \
        '{family: $family, target: $target, architecture: $architecture,
          elf: {path: $elf, sha256: $elf_sha256},
          replay: {result: "pass", byte_identical: true, path: $run, sha256: $run_sha256},
          coverage: {path: $coverage, sha256: $coverage_sha256,
            fetch_accesses: $coverage_data[0].fetch_accesses,
            unique_addresses: $coverage_data[0].unique_addresses,
            digest: $coverage_data[0].digest},
          gdb: {result: "pass", session: $session, session_sha256: $session_sha256,
            transcript: $transcript, transcript_sha256: $transcript_sha256,
            report: $session_data[0].report}}' >"$case_root/summary.json"
}

qualify riscv ch32v003 .remu/portfolio-smoke/wch/smoke.elf riscv:rv32
qualify arm rp2040 .remu/portfolio-smoke/rp-arm/smoke.elf arm
qualify xtensa esp32s3 .remu/portfolio-smoke/esp32s3/smoke.elf xtensa

source_sha=$(sha256sum \
    crates/remu-gdb/src/lib.rs \
    crates/remu-cli/src/main.rs \
    crates/remu-machines/src/riscv.rs \
    crates/remu-machines/src/arm.rs \
    crates/remu-machines/src/xtensa.rs \
    qualification/gdb_client.py \
    scripts/qualify-debug-observability.sh | sha256sum | cut -d ' ' -f 1)

jq -s \
    --arg schema remu.debug-observability-qualification.v1 \
    --arg generated_by scripts/qualify-debug-observability.sh \
    --arg source_sha256 "$source_sha" \
    '{schema: $schema, generated_by: $generated_by, source_sha256: $source_sha256,
      result: "pass", families: .}' \
    "$root/riscv/summary.json" "$root/arm/summary.json" "$root/xtensa/summary.json" >"$artifact"

jq -e '.result == "pass" and (.families | length == 3) and ([.families[].replay.result] | all(. == "pass")) and ([.families[].gdb.result] | all(. == "pass"))' "$artifact" >/dev/null
echo "GDB, coverage, and replay passed on three CPU families; artifact: $artifact"
