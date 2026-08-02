#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:744a8f397347c3a5b0a6448e55e8257f69aa24c51ecacc3be49c37284719ef7c remu-xc8:4.00-pic16f1xxxx-1.31.465)
artifact_root=${PIC16F15376_ARTIFACT_ROOT:-.remu/qualification/pic16f15376}
toolchain=toolchains/xc8-pic16f15376.toml

docker image inspect "$image" >/dev/null
cargo build -q -p remu-cli
cargo test -q -p remu-cpu-pic16 -p remu-image
remu=target/debug/remu
mkdir -p "$artifact_root"

expected_uart='[80,73,67,49,54,70,49,53,51,55,54,58,79,75,10,73,82,81,10]'

for optimization in O0 Os O2; do
    build="$artifact_root/build-$optimization"
    run="$artifact_root/run-$optimization"
    mkdir -p "$build" "$run"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/pic16f15376 \
        --output "$build" \
        --target pic16f15376 \
        --artifact "$artifact_root/build-$optimization.json" \
        -- "-$optimization" main.c \
        -Wl,-Map=/workspace/out/smoke.map \
        -o /workspace/out/smoke.elf
    docker run --rm --network=none \
        -v "$repo_root/$build:/workspace/out" \
        "$image" pic-objdump -d -S /workspace/out/smoke.elf \
        >"$build/smoke.disasm"
    "$remu" run \
        --target pic16f15376 \
        --hex "$build/smoke.hex" \
        --max-instructions 30000 \
        --pin 1=1@0 \
        --vcd "$run/signals.vcd" \
        --bus-log "$run/bus.json" \
        --coverage "$run/coverage.json" \
        --result "$run/result.json"
    jq -e --argjson uart "$expected_uart" \
        '.target == "pic16f15376" and .reason == "InstructionLimit" and
         .exit_code == 0 and .uart == $uart and
         .cpu.architecture == "Pic16Enhanced"' \
        "$run/result.json" >/dev/null
done

mkdir -p "$artifact_root/run-O2-repeat"
"$remu" run \
    --target pic16f15376 \
    --hex "$artifact_root/build-O2/smoke.hex" \
    --max-instructions 30000 \
    --pin 1=1@0 \
    --vcd "$artifact_root/run-O2-repeat/signals.vcd" \
    --replay "$artifact_root/run-O2/result.json" \
    --result "$artifact_root/run-O2-repeat/result.json"
cmp "$artifact_root/run-O2/result.json" "$artifact_root/run-O2-repeat/result.json"
cmp "$artifact_root/run-O2/signals.vcd" "$artifact_root/run-O2-repeat/signals.vcd"
grep -q '^\$scope module pic16f15376 \$end$' "$artifact_root/run-O2/signals.vcd"
grep -q '^\$scope module timer0 \$end$' "$artifact_root/run-O2/signals.vcd"
grep -q '^\$scope module eusart1 \$end$' "$artifact_root/run-O2/signals.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/run-O2/signals.vcd"
grep -q 'porta' "$artifact_root/run-O2/signals.vcd"
grep -q 'timer0' "$artifact_root/run-O2/signals.vcd"
grep -q 'eusart1' "$artifact_root/run-O2/signals.vcd"
grep -q 'interrupt' "$artifact_root/run-O2/signals.vcd"
jq -e '.architecture == "Pic16Enhanced" and .fetch_accesses > 100 and .unique_addresses > 100' \
    "$artifact_root/run-O2/coverage.json" >/dev/null

isa_build="$artifact_root/isa-build"
mkdir -p "$isa_build"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/isa \
    --output "$isa_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/isa-build.json" \
    -- -c all_instructions.s \
    -Wa,-a=/workspace/out/all_instructions.lst \
    -o /workspace/out/all_instructions.o
test "$(awk '$1 ~ /^[0-9]+$/ && $1 >= 7 && $1 <= 55 { count++ } END { print count }' \
    "$isa_build/all_instructions.lst")" = 49

fixture_build="$artifact_root/register-timer0-build"
fixture_run="$artifact_root/register-timer0-run"
mkdir -p "$fixture_build" "$fixture_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/fixture \
    --output "$fixture_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/register-timer0-build.json" \
    -- -Os remu_timer0.c \
    -Wl,-Map=/workspace/out/timer0.map \
    -o /workspace/out/timer0.elf
docker run --rm --network=none \
    -v "$repo_root/$fixture_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/timer0.elf \
    >"$fixture_build/timer0.disasm"
"$remu" run \
    --target pic16f15376 \
    --hex "$fixture_build/timer0.hex" \
    --max-instructions 30000 \
    --stop-signal board.pic16f15376.porte.pin0=rising \
    --vcd "$fixture_run/signals.vcd" \
    --bus-log "$fixture_run/bus.json" \
    --result "$fixture_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.porte.pin0"}' \
    "$fixture_run/result.json" >/dev/null
grep -q 'timer0' "$fixture_run/signals.vcd"
grep -q 'interrupt' "$fixture_run/signals.vcd"
grep -q 'porte' "$fixture_run/signals.vcd"

timer2_build="$artifact_root/register-timer2-build"
timer2_run="$artifact_root/register-timer2-run"
mkdir -p "$timer2_build" "$timer2_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/fixture \
    --output "$timer2_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/register-timer2-build.json" \
    -- -Os remu_timer2.c \
    -Wl,-Map=/workspace/out/timer2.map \
    -o /workspace/out/timer2.elf
docker run --rm --network=none \
    -v "$repo_root/$timer2_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/timer2.elf \
    >"$timer2_build/timer2.disasm"
"$remu" run \
    --target pic16f15376 \
    --hex "$timer2_build/timer2.hex" \
    --max-instructions 30000 \
    --stop-signal board.pic16f15376.porte.pin0=rising \
    --vcd "$timer2_run/signals.vcd" \
    --bus-log "$timer2_run/bus.json" \
    --result "$timer2_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.porte.pin0"}' \
    "$timer2_run/result.json" >/dev/null
grep -q 'timer2' "$timer2_run/signals.vcd"
grep -q 'interrupt' "$timer2_run/signals.vcd"
grep -q 'porte' "$timer2_run/signals.vcd"

dac_build="$artifact_root/register-dac-build"
dac_run="$artifact_root/register-dac-run"
mkdir -p "$dac_build" "$dac_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/fixture \
    --output "$dac_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/register-dac-build.json" \
    -- -Os remu_dac.c \
    -Wl,-Map=/workspace/out/dac.map \
    -o /workspace/out/dac.elf
docker run --rm --network=none \
    -v "$repo_root/$dac_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/dac.elf \
    >"$dac_build/dac.disasm"
"$remu" run \
    --target pic16f15376 \
    --hex "$dac_build/dac.hex" \
    --max-instructions 30000 \
    --stop-signal board.pic16f15376.dac1.active=rising \
    --vcd "$dac_run/signals.vcd" \
    --bus-log "$dac_run/bus.json" \
    --result "$dac_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.dac1.active"}' \
    "$dac_run/result.json" >/dev/null
grep -q 'dac1' "$dac_run/signals.vcd"

comparator_build="$artifact_root/register-comparator-build"
comparator_run="$artifact_root/register-comparator-run"
mkdir -p "$comparator_build" "$comparator_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/fixture \
    --output "$comparator_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/register-comparator-build.json" \
    -- -Os remu_comparator.c \
    -Wl,-Map=/workspace/out/comparator.map \
    -o /workspace/out/comparator.elf
docker run --rm --network=none \
    -v "$repo_root/$comparator_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/comparator.elf \
    >"$comparator_build/comparator.disasm"
"$remu" run \
    --target pic16f15376 \
    --hex "$comparator_build/comparator.hex" \
    --max-instructions 30000 \
    --pin 0=0@0 \
    --pin 2=1@0 \
    --stop-signal board.pic16f15376.comparator1.output=rising \
    --vcd "$comparator_run/signals.vcd" \
    --bus-log "$comparator_run/bus.json" \
    --result "$comparator_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.comparator1.output"}' \
    "$comparator_run/result.json" >/dev/null
grep -q 'comparator1' "$comparator_run/signals.vcd"

pps_build="$artifact_root/register-pps-build"
pps_run="$artifact_root/register-pps-run"
mkdir -p "$pps_build" "$pps_run"
"$remu" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/fixture \
    --output "$pps_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/register-pps-build.json" \
    -- -Os remu_pps.c \
    -Wl,-Map=/workspace/out/pps.map \
    -o /workspace/out/pps.elf
docker run --rm --network=none \
    -v "$repo_root/$pps_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/pps.elf \
    >"$pps_build/pps.disasm"
"$remu" run \
    --target pic16f15376 \
    --hex "$pps_build/pps.hex" \
    --max-instructions 30000 \
    --stop-signal board.pic16f15376.porta.pin0=rising \
    --vcd "$pps_run/signals.vcd" \
    --bus-log "$pps_run/bus.json" \
    --result "$pps_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.porta.pin0"}' \
    "$pps_run/result.json" >/dev/null
grep -q 'porta' "$pps_run/signals.vcd"

find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "PIC16F15376 Docker/offline qualification passed: $artifact_root"
