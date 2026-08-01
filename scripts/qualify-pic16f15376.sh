#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

image=sha256:744a8f397347c3a5b0a6448e55e8257f69aa24c51ecacc3be49c37284719ef7c
artifact_root=${PIC16F15376_ARTIFACT_ROOT:-.renvo/qualification/pic16f15376}
toolchain=toolchains/xc8-pic16f15376.toml

docker image inspect "$image" >/dev/null
PATH=/home/joshua/.cargo/bin:$PATH cargo build -q -p renvo-cli
PATH=/home/joshua/.cargo/bin:$PATH cargo test -q -p renvo-cpu-pic16 -p renvo-image
renvo=target/debug/renvo
mkdir -p "$artifact_root"

expected_uart='[80,73,67,49,54,70,49,53,51,55,54,58,79,75,10,73,82,81,10]'

for optimization in O0 Os O2; do
    build="$artifact_root/build-$optimization"
    run="$artifact_root/run-$optimization"
    mkdir -p "$build" "$run"
    "$renvo" corpus build \
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
    "$renvo" run \
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
"$renvo" run \
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
"$renvo" corpus build \
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

official_build="$artifact_root/microchip-timer0-build"
official_run="$artifact_root/microchip-timer0-run"
mkdir -p "$official_build" "$official_run"
"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source qualification/pic16f15376/microchip \
    --output "$official_build" \
    --target pic16f15376 \
    --artifact "$artifact_root/microchip-timer0-build.json" \
    -- -Os timer0_periodic.c \
    -Wl,-Map=/workspace/out/timer0.map \
    -o /workspace/out/timer0.elf
docker run --rm --network=none \
    -v "$repo_root/$official_build:/workspace/out" \
    "$image" pic-objdump -d -S /workspace/out/timer0.elf \
    >"$official_build/timer0.disasm"
"$renvo" run \
    --target pic16f15376 \
    --hex "$official_build/timer0.hex" \
    --max-instructions 30000 \
    --stop-signal board.pic16f15376.porte.pin0=rising \
    --vcd "$official_run/signals.vcd" \
    --bus-log "$official_run/bus.json" \
    --result "$official_run/result.json"
jq -e '.target == "pic16f15376" and
       .reason == {"Signal":"board.pic16f15376.porte.pin0"}' \
    "$official_run/result.json" >/dev/null
grep -q 'timer0' "$official_run/signals.vcd"
grep -q 'interrupt' "$official_run/signals.vcd"
grep -q 'porte' "$official_run/signals.vcd"

find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum \
    >"$artifact_root/SHA256SUMS"

echo "PIC16F15376 Docker/offline qualification passed: $artifact_root"
