#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 remu/cross-gcc:local)
clang_image=$(resolve_toolchain_image sha256:e31d07e59ec7eb7f05396e787db55127783b8a21bc2e907ebcaef6534d343dac remu/cross-llvm:local)
artifact_root=${RA4M1_ARTIFACT_ROOT:-.remu/qualification/r7fa4m1ab3cfm}
toolchain=toolchains/arm-gcc-r7fa4m1ab3cfm.toml
clang_toolchain=toolchains/arm-clang-r7fa4m1ab3cfm.toml

docker image inspect "$image" >/dev/null
docker image inspect "$clang_image" >/dev/null
cargo build -q -p remu-cli
remu=target/debug/remu
mkdir -p "$artifact_root/build" "$artifact_root/run-a" "$artifact_root/run-b"

"$remu" corpus build \
    --toolchain "$toolchain" \
    --source corpus/smoke/r7fa4m1ab3cfm \
    --output "$artifact_root/build" \
    --target r7fa4m1ab3cfm \
    --artifact "$artifact_root/build.json" \
    -- -O2 -mfpu=fpv4-sp-d16 -mfloat-abi=hard start.S main.c \
    -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -o /workspace/out/smoke.elf

build_path=$(CDPATH= cd -- "$artifact_root/build" && pwd)
docker run --rm --network=none --pull=never --read-only \
    --entrypoint arm-none-eabi-objdump --volume "$build_path:/input:ro" \
    "$image" -d /input/smoke.elf > "$artifact_root/build/smoke.disassembly"
docker run --rm --network=none --pull=never --read-only \
    --entrypoint arm-none-eabi-objcopy --volume "$build_path:/work" \
    "$image" -O ihex /work/smoke.elf /work/smoke.hex

run_once()
{
    output=$1
    "$remu" run \
        --target r7fa4m1ab3cfm \
        --elf "$artifact_root/build/smoke.elf" \
        --max-instructions 20000 \
        --pin 3=0@0 \
        --pin 3=1@200 \
        --vcd "$output/gpio.vcd" \
        --bus-log "$output/bus.json" \
        --result "$output/result.json"
}

run_once "$artifact_root/run-a"
run_once "$artifact_root/run-b"
cmp "$artifact_root/run-a/result.json" "$artifact_root/run-b/result.json"
cmp "$artifact_root/run-a/gpio.vcd" "$artifact_root/run-b/gpio.vcd"
jq -e '.target == "r7fa4m1ab3cfm" and .reason == "Halted" and .exit_code == 0 and (.uart == [82,65,52,77,49,10])' \
    "$artifact_root/run-a/result.json" >/dev/null
grep -q '^\$scope module port1 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module gpt0 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module kint \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module sci9 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module icu \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^1' "$artifact_root/run-a/gpio.vcd"
grep -q '^0' "$artifact_root/run-a/gpio.vcd"
grep -q 'vmul\.f32' "$artifact_root/build/smoke.disassembly"
test -s "$artifact_root/build/smoke.hex"

for optimization in O0 Os; do
    mkdir -p "$artifact_root/build-$optimization" "$artifact_root/run-$optimization"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/r7fa4m1ab3cfm \
        --output "$artifact_root/build-$optimization" \
        --target r7fa4m1ab3cfm \
        --artifact "$artifact_root/build-$optimization.json" \
        -- "-$optimization" -mfpu=fpv4-sp-d16 -mfloat-abi=hard start.S main.c \
        -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -o /workspace/out/smoke.elf
    "$remu" run \
        --target r7fa4m1ab3cfm \
        --elf "$artifact_root/build-$optimization/smoke.elf" \
        --max-instructions 30000 \
        --pin 3=1@0 \
        --vcd "$artifact_root/run-$optimization/gpio.vcd" \
        --result "$artifact_root/run-$optimization/result.json"
    jq -e '.target == "r7fa4m1ab3cfm" and .reason == "Halted" and .exit_code == 0 and (.uart == [82,65,52,77,49,10])' \
        "$artifact_root/run-$optimization/result.json" >/dev/null
done

mkdir -p "$artifact_root/clang-build" "$artifact_root/clang-run"
"$remu" corpus build \
    --toolchain "$clang_toolchain" \
    --source corpus/smoke/r7fa4m1ab3cfm \
    --output "$artifact_root/clang-build" \
    --target r7fa4m1ab3cfm \
    --artifact "$artifact_root/clang-build.json" \
    -- -O2 start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map \
    -lgcc -o /workspace/out/smoke.elf
"$remu" run \
    --target r7fa4m1ab3cfm \
    --elf "$artifact_root/clang-build/smoke.elf" \
    --max-instructions 30000 \
    --pin 3=1@0 \
    --vcd "$artifact_root/clang-run/gpio.vcd" \
    --result "$artifact_root/clang-run/result.json"
jq -e '.target == "r7fa4m1ab3cfm" and .reason == "Halted" and .exit_code == 0 and (.uart == [82,65,52,77,49,10])' \
    "$artifact_root/clang-run/result.json" >/dev/null

fsp_revision=a409855a274402f69360a725656944e17929d1d9
fsp_examples_revision=01a411dfc2e9808f489070c780a554a5bead6714
arduino_core_revision=424e86eff92d37f72123c2b641dd8bbf06a38b47
arduino_examples_revision=ad14bc44cb95555e5df7c16e6605559cad860d29
test "$(git -C .remu/upstream/fsp rev-parse HEAD)" = "$fsp_revision"
test "$(git -C .remu/upstream/ra-fsp-examples rev-parse HEAD)" = "$fsp_examples_revision"
test "$(git -C .remu/upstream/ArduinoCore-renesas rev-parse HEAD)" = "$arduino_core_revision"
test "$(git -C .remu/upstream/arduino-examples rev-parse HEAD)" = "$arduino_examples_revision"

fsp_stage="$artifact_root/fsp-source"
mkdir -p "$fsp_stage" "$artifact_root/fsp-build" "$artifact_root/fsp-run"
cp qualification/r7fa4m1ab3cfm/fsp/common_utils.h "$fsp_stage/common_utils.h"
cp qualification/r7fa4m1ab3cfm/fsp/gpt_timer.h "$fsp_stage/gpt_timer.h"
cp qualification/r7fa4m1ab3cfm/fsp/uart_ep.h "$fsp_stage/uart_ep.h"
cp qualification/r7fa4m1ab3cfm/fsp/timer_pwm.h "$fsp_stage/timer_pwm.h"
cp qualification/r7fa4m1ab3cfm/fsp/adapter.c "$fsp_stage/adapter.c"
cp qualification/r7fa4m1ab3cfm/fsp/harness.c "$fsp_stage/harness.c"
cp qualification/r7fa4m1ab3cfm/fsp/start.S "$fsp_stage/start.S"
cp qualification/r7fa4m1ab3cfm/fsp/link.ld "$fsp_stage/link.ld"
cp .remu/upstream/ra-fsp-examples/example_projects/ek_ra4m1/gpt/gpt_ek_ra4m1_ep/e2studio/src/gpt_timer.c \
    "$fsp_stage/gpt_timer.c"
cp .remu/upstream/ra-fsp-examples/example_projects/ek_ra4m1/sci_uart/sci_uart_ek_ra4m1_ep/e2studio/src/uart_ep.c \
    "$fsp_stage/uart_ep.c"

"$remu" corpus build \
    --toolchain "$toolchain" \
    --source "$fsp_stage" \
    --output "$artifact_root/fsp-build" \
    --target r7fa4m1ab3cfm \
    --artifact "$artifact_root/fsp-build.json" \
    -- -O2 -I. -ffunction-sections -fdata-sections start.S gpt_timer.c uart_ep.c \
    adapter.c harness.c -Wl,-T,link.ld,--gc-sections,-Map,/workspace/out/fsp.map \
    -o /workspace/out/fsp.elf
"$remu" run \
    --target r7fa4m1ab3cfm \
    --elf "$artifact_root/fsp-build/fsp.elf" \
    --max-instructions 20000 \
    --stop-signal board.r7fa4m1ab3cfm.port1.pin11=rising \
    --vcd "$artifact_root/fsp-run/signals.vcd" \
    --result "$artifact_root/fsp-run/result.json"
jq -e '.target == "r7fa4m1ab3cfm" and (.reason.Signal == "board.r7fa4m1ab3cfm.port1.pin11") and (.uart == [70,83,80,32,71,80,84,32,83,67,73,10])' \
    "$artifact_root/fsp-run/result.json" >/dev/null

qualify_arduino()
{
    name=$1
    sketch=$2
    source_dir="$artifact_root/arduino-$name-source"
    build_dir="$artifact_root/arduino-$name-build"
    run_dir="$artifact_root/arduino-$name-run"
    mkdir -p "$source_dir" "$build_dir" "$run_dir"
    cp qualification/r7fa4m1ab3cfm/arduino/Arduino.h "$source_dir/Arduino.h"
    cp qualification/r7fa4m1ab3cfm/arduino/adapter.cpp "$source_dir/adapter.cpp"
    cp qualification/r7fa4m1ab3cfm/arduino/harness.cpp "$source_dir/harness.cpp"
    cp qualification/r7fa4m1ab3cfm/arduino/start.S "$source_dir/start.S"
    cp qualification/r7fa4m1ab3cfm/arduino/link.ld "$source_dir/link.ld"
    cp "$sketch" "$source_dir/sketch.cpp"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "$source_dir" \
        --output "$build_dir" \
        --target r7fa4m1ab3cfm \
        --artifact "$artifact_root/arduino-$name-build.json" \
        -- -O2 -x none -I. -include Arduino.h -fno-exceptions -fno-rtti \
        -fno-use-cxa-atexit start.S sketch.cpp adapter.cpp harness.cpp \
        -Wl,-T,link.ld,--gc-sections,-Map,/workspace/out/sketch.map \
        -o /workspace/out/sketch.elf
}

qualify_arduino blink \
    .remu/upstream/arduino-examples/examples/01.Basics/Blink/Blink.ino
"$remu" run \
    --target r7fa4m1ab3cfm \
    --elf "$artifact_root/arduino-blink-build/sketch.elf" \
    --max-instructions 10000 \
    --stop-signal board.r7fa4m1ab3cfm.port1.pin11=rising \
    --vcd "$artifact_root/arduino-blink-run/gpio.vcd" \
    --result "$artifact_root/arduino-blink-run/result.json"
jq -e '.target == "r7fa4m1ab3cfm" and (.reason.Signal == "board.r7fa4m1ab3cfm.port1.pin11")' \
    "$artifact_root/arduino-blink-run/result.json" >/dev/null

qualify_arduino hardware-serial \
    .remu/upstream/arduino-examples/examples/04.Communication/MultiSerial/MultiSerial.ino
"$remu" run \
    --target r7fa4m1ab3cfm \
    --elf "$artifact_root/arduino-hardware-serial-build/sketch.elf" \
    --max-instructions 10000 \
    --vcd "$artifact_root/arduino-hardware-serial-run/signals.vcd" \
    --result "$artifact_root/arduino-hardware-serial-run/result.json"
jq -e '.target == "r7fa4m1ab3cfm" and .reason == "Halted" and .exit_code == 0 and (.uart == [72])' \
    "$artifact_root/arduino-hardware-serial-run/result.json" >/dev/null

echo "R7FA4M1AB3CFM Docker/offline smoke qualification passed: $artifact_root"
