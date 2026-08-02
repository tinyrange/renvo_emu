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
grep -q '^\$scope module adc0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module comparator0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module comparator1 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module clu0 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module clu1 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module clu2 \$end$' "$artifact_root/run-speed/signals.vcd"
grep -q '^\$scope module clu3 \$end$' "$artifact_root/run-speed/signals.vcd"
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
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$uart_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-uart1-build.json" \
    -- -I. -c remu_uart1_irq.c -o /workspace/out/uart1.rel
test -s "$uart_build/uart1.rel"

adc_build="$artifact_root/register-adc-build"
mkdir -p "$adc_build"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$adc_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-adc-build.json" \
    -- -I. -c remu_adc_irq.c -o /workspace/out/adc.rel
test -s "$adc_build/adc.rel"

comparator_build="$artifact_root/register-comparator-build"
mkdir -p "$comparator_build"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$comparator_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-comparator-build.json" \
    -- -I. -c remu_comparator.c -o /workspace/out/comparator.rel
test -s "$comparator_build/comparator.rel"

clu_build="$artifact_root/register-clu-build"
clu_run="$artifact_root/register-clu-run"
mkdir -p "$clu_build" "$clu_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$clu_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-clu-build.json" \
    -- -I. -c remu_clu.c -o /workspace/out/clu.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$clu_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-clu-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$clu_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-clu-link.json" \
    -- /workspace/out/clu.rel /workspace/out/adapter.rel -o /workspace/out/clu.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$clu_build/clu.ihx" \
    --max-instructions 10000 \
    --pin 0=1@0 \
    --pin 1=1@0 \
    --stop-signal board.efm8bb52f32g.port1.pin4=rising \
    --vcd "$clu_run/signals.vcd" \
    --result "$clu_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.port1.pin4"}' \
    "$clu_run/result.json" >/dev/null
grep -q '^\$scope module clu0 \$end$' "$clu_run/signals.vcd"
test -s "$clu_build/clu.rel"

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

dac_build="$artifact_root/register-dac-build"
dac_run="$artifact_root/register-dac-run"
mkdir -p "$dac_build" "$dac_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$dac_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-dac-build.json" \
    -- -I. -c remu_dac.c -o /workspace/out/dac.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$dac_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-dac-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$dac_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-dac-link.json" \
    -- /workspace/out/dac.rel /workspace/out/adapter.rel -o /workspace/out/dac.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$dac_build/dac.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.dac0.output=change \
    --vcd "$dac_run/signals.vcd" \
    --result "$dac_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.dac0.output"}' \
    "$dac_run/result.json" >/dev/null
grep -q '^\$scope module dac0 \$end$' "$dac_run/signals.vcd"

flash_build="$artifact_root/flash-build"
flash_run="$artifact_root/flash-run"
mkdir -p "$flash_build" "$flash_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$flash_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-flash-build.json" \
    -- -I. -c remu_flash.c -o /workspace/out/flash.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$flash_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-flash-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$flash_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-flash-link.json" \
    -- /workspace/out/flash.rel /workspace/out/adapter.rel -o /workspace/out/flash.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$flash_build/flash.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.port1.pin4=change \
    --vcd "$flash_run/signals.vcd" \
    --result "$flash_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.port1.pin4"} and
       .exit_code == 0' \
    "$flash_run/result.json" >/dev/null
grep -q '^$scope module port1 $end$' "$flash_run/signals.vcd"

crossbar_build="$artifact_root/crossbar-build"
crossbar_run="$artifact_root/crossbar-run"
mkdir -p "$crossbar_build" "$crossbar_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$crossbar_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-crossbar-build.json" \
    -- -I. -c remu_crossbar.c -o /workspace/out/crossbar.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$crossbar_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-crossbar-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$crossbar_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/register-crossbar-link.json" \
    -- /workspace/out/crossbar.rel /workspace/out/adapter.rel -o /workspace/out/crossbar.ihx
"$remu" run \
    --target efm8bb52f32g \
    --hex "$crossbar_build/crossbar.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.crossbar.uart0.tx_pin=change \
    --vcd "$crossbar_run/signals.vcd" \
    --result "$crossbar_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.crossbar.uart0.tx_pin"} and
       .exit_code == 0' \
    "$crossbar_run/result.json" >/dev/null
grep -q '^$scope module crossbar $end$' "$crossbar_run/signals.vcd"
grep -q 'uart0' "$crossbar_run/signals.vcd"

docker inspect "$image" >"$artifact_root/toolchain-image.json"
sha256sum toolchains/sdcc-mcs51/Dockerfile >"$artifact_root/Dockerfile.sha256"
find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "EFM8BB52F32G Docker/offline qualification passed: $artifact_root"
