#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

image=sha256:ba8ecc7e43912412757079d7366e5f27b78a272dc08d61a244b79508139dbf75
artifact_root=${EFM8BB52F32G_ARTIFACT_ROOT:-.renvo/qualification/efm8bb52f32g}
toolchain=toolchains/sdcc-mcs51-efm8bb52.toml

docker image inspect "$image" >/dev/null
PATH=/home/joshua/.cargo/bin:$PATH cargo build -q -p renvo-cli
PATH=/home/joshua/.cargo/bin:$PATH cargo test -q -p renvo-cpu-mcs51 -p renvo-image
renvo=target/debug/renvo
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
    "$renvo" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/efm8bb52f32g \
        --output "$build" \
        --target efm8bb52f32g \
        --artifact "$artifact_root/build-$profile.json" \
        -- $optimization_flags main.c -o /workspace/out/smoke.ihx
    "$renvo" run \
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
"$renvo" run \
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

official="$artifact_root/silabs-blinky-build"
official_run="$artifact_root/silabs-blinky-run"
mkdir -p "$official" "$official_run"
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$official" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/silabs-main-build.json" \
    -- -I. -c EFM8BB52_Blinky.c -o /workspace/out/main.rel
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$official" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/silabs-isr-build.json" \
    -- -I. -c Interrupts.c -o /workspace/out/interrupts.rel
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$official" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/silabs-adapter-build.json" \
    -- -I. -c InitDevice_adapter.c -o /workspace/out/adapter.rel
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$official" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/silabs-link.json" \
    -- /workspace/out/main.rel /workspace/out/interrupts.rel \
    /workspace/out/adapter.rel -o /workspace/out/blinky.ihx
"$renvo" run \
    --target efm8bb52f32g \
    --hex "$official/blinky.ihx" \
    --max-instructions 10000 \
    --stop-signal board.efm8bb52f32g.port1.pin4=change \
    --vcd "$official_run/signals.vcd" \
    --bus-log "$official_run/bus.json" \
    --result "$official_run/result.json"
jq -e '.target == "efm8bb52f32g" and
       .reason == {"Signal":"board.efm8bb52f32g.port1.pin4"}' \
    "$official_run/result.json" >/dev/null
grep -q '^\$scope module timer2 \$end$' "$official_run/signals.vcd"
grep -q '^\$scope module port1 \$end$' "$official_run/signals.vcd"

uart_build="$artifact_root/silabs-uart-build"
mkdir -p "$uart_build"
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/efm8bb52f32g/silabs \
    --output "$uart_build" \
    --target efm8bb52f32g \
    --artifact "$artifact_root/silabs-uart-build.json" \
    -- -I. -c UART_Interrupts.c -o /workspace/out/uart.rel
test -s "$uart_build/uart.rel"

docker inspect "$image" >"$artifact_root/toolchain-image.json"
sha256sum toolchains/sdcc-mcs51/Dockerfile >"$artifact_root/Dockerfile.sha256"
find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "EFM8BB52F32G Docker/offline qualification passed: $artifact_root"
