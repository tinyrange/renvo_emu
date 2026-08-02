#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:ba8ecc7e43912412757079d7366e5f27b78a272dc08d61a244b79508139dbf75 remu/sdcc-mcs51:4.5.0)
artifact_root=${EFM8BB52F32G_ARTIFACT_ROOT:-.remu/qualification/efm8bb52f32g}
toolchain=toolchains/sdcc-mcs51-efm8bb52.toml

docker image inspect "$image" >/dev/null
cargo build -q -p remu-cli
cargo test -q -p remu-cpu-mcs51 -p remu-image
remu=target/debug/remu
mkdir -p "$artifact_root"

expected_uart='[69,70,77,56,66,66,53,50,58,79,75,10,73,82,81,10]'

for profile in baseline size speed; do
    build="$artifact_root/build-$profile"
    run="$artifact_root/run-$profile"
    mkdir -p "$build" "$run"
    case "$profile" in
        baseline) optimization_flags= ;;
        size) optimization_flags=--opt-code-size ;;
        speed) optimization_flags=--opt-code-speed ;;
    esac
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/efm8bb52f32g \
        --output "$build" \
        --target efm8bb52f32g \
        --artifact "$artifact_root/build-$profile.json" \
        -- $optimization_flags main.c -o /workspace/out/smoke.ihx
    "$remu" run \
        --target efm8bb52f32g \
        --hex "$build/smoke.ihx" \
        --max-instructions 30000 \
        --pin 1=1@0 \
        --vcd "$run/signals.vcd" \
        --bus-log "$run/bus.json" \
        --coverage "$run/coverage.json" \
        --result "$run/result.json"
    jq -e --argjson uart "$expected_uart" \
        '.target == "efm8bb52f32g" and .reason == "InstructionLimit" and
         .exit_code == 0 and .uart == $uart and
         .cpu.architecture == "Mcs51"' \
        "$run/result.json" >/dev/null
done

mkdir -p "$artifact_root/run-speed-repeat"
"$remu" run \
    --target efm8bb52f32g \
    --hex "$artifact_root/build-speed/smoke.ihx" \
    --max-instructions 30000 \
    --pin 1=1@0 \
    --vcd "$artifact_root/run-speed-repeat/signals.vcd" \
    --replay "$artifact_root/run-speed/result.json" \
    --result "$artifact_root/run-speed-repeat/result.json"
cmp "$artifact_root/run-speed/result.json" "$artifact_root/run-speed-repeat/result.json"
cmp "$artifact_root/run-speed/signals.vcd" "$artifact_root/run-speed-repeat/signals.vcd"
grep -q '^\$scope module efm8bb52f32g \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module port0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module timer0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module timer2 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module uart0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/run-speed/signals.vcd"
jq -e '.architecture == "Mcs51" and .fetch_accesses > 100 and .unique_addresses > 100' \
    "$artifact_root/run-speed/coverage.json" >/dev/null

fixture="$artifact_root/register-fixture-build"
fixture_run="$artifact_root/register-fixture-run"
mkdir -p "$fixture" "$fixture_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$fixture" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-main-build.json" \
    -- -I. -c remu_blinky.c -o /workspace/out/main.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$fixture" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-isr-build.json" \
    -- -I. -c remu_timer2_irq.c -o /workspace/out/interrupts.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$fixture" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$fixture" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-link.json" \
    -- /workspace/out/main.rel /workspace/out/interrupts.rel \
    /workspace/out/adapter.rel -o /workspace/out/blinky.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$fixture/blinky.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.port1.pin4=change \
    --vcd "$fixture_run/signals.vcd" \
    --bus-log "$fixture_run/bus.json" \
    --result "$fixture_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.port1.pin4"}' \
    "$fixture_run/result.json" >/dev/null
grep -q '^\$scope module timer2 \$end$' "$fixture_run/signals.vcd"
grep -q '^\$scope module port1 \$end$' "$fixture_run/signals.vcd"

port_match_build="$artifact_root/register-port-match-build"
port_match_run="$artifact_root/register-port-match-run"
mkdir -p "$port_match_build" "$port_match_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$port_match_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-port-match-build.json" \
    -- -I. -c remu_port_match.c -o /workspace/out/port_match.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$port_match_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-port-match-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$port_match_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-port-match-link.json" \
    -- /workspace/out/port_match.rel /workspace/out/adapter.rel -o /workspace/out/port_match.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$port_match_build/port_match.ihx" \
    --max-instructions 10000 \
    --pin 0=0@0 \
    --stop-signal board.efm8bb52f32g.port_match.event=rising \
    --vcd "$port_match_run/signals.vcd" \
    --result "$port_match_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.port_match.event"}' \
    "$port_match_run/result.json" >/dev/null
grep -q '^\$scope module port_match \$end$' "$port_match_run/signals.vcd"
test -s "$port_match_build/port_match.rel"

uart_build="$artifact_root/register-uart-build"
mkdir -p "$uart_build"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$uart_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-uart-build.json" \
    -- -I. -c remu_uart_irq.c -o /workspace/out/uart.rel
test -s "$uart_build/uart.rel"

docker inspect "$image" >"$artifact_root/toolchain-image.json"
sha256sum toolchains/sdcc-mcs51/Dockerfile >"$artifact_root/Dockerfile.sha256"
find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "EFM8BB52F32G Docker/offline qualification passed: $artifact_root"
