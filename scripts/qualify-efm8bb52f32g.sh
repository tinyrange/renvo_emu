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
cargo test -q -p remu-cpu-mcs51 -p remu-devices -p remu-image
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
grep -q '^\$scope module timer3 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module timer4 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module timer5 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module uart0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module pca0 \$end$' "$artifact_root/run-speed/signals.vcd"
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
    --artifact "$artifact_root/register-timer345-build.json" \
    -- -I. -c remu_timer345_irq.c -o /workspace/out/timer345.rel
test -s "$fixture/timer345.rel"
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
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$uart_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-uart1-build.json" \
    -- -I. -c remu_uart1_irq.c -o /workspace/out/uart1.rel
test -s "$uart_build/uart1.rel"

pca_build="$artifact_root/register-pca-build"
mkdir -p "$pca_build"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$pca_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-pca-build.json" \
    -- -I. -c remu_pca.c -o /workspace/out/pca.rel
test -s "$pca_build/pca.rel"

smbus_build="$artifact_root/register-smbus-build"
smbus_run="$artifact_root/register-smbus-run"
mkdir -p "$smbus_build" "$smbus_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$smbus_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-smbus-source-build.json" \
    -- -I. -c remu_smbus.c -o /workspace/out/smbus.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$smbus_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-smbus-link.json" \
    -- /workspace/out/smbus.rel -o /workspace/out/smbus.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$smbus_build/smbus.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.smb0.tx_strobe=rising \
    --vcd "$smbus_run/signals.vcd" \
    --bus-log "$smbus_run/bus.json" \
    --result "$smbus_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.smb0.tx_strobe"}' \
    "$smbus_run/result.json" >/dev/null
grep -q 'smb0' "$smbus_run/signals.vcd"

docker inspect "$image" >"$artifact_root/toolchain-image.json"
sha256sum toolchains/sdcc-mcs51/Dockerfile >"$artifact_root/Dockerfile.sha256"
find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "EFM8BB52F32G Docker/offline qualification passed: $artifact_root"
