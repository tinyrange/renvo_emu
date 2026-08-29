#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

arm_image=$(resolve_toolchain_image sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 remu/cross-gcc:local)
riscv_image=$(resolve_toolchain_image sha256:634d2ca3820e772fe162a100465589cd57dbb1a387b9df4175c055ef360f90f6 remu/riscv32-esp-gcc:local)
docker image inspect "$arm_image" >/dev/null
docker image inspect "$riscv_image" >/dev/null

artifact_root=${REMU_NEW_TARGET_ARTIFACT_ROOT:-.remu/qualification/new-targets}
mkdir -p "$artifact_root"
cargo build -q -p remu-cli
remu=target/debug/remu

build_arm()
{
    target=$1
    toolchain=$2
    mkdir -p "$artifact_root/$target-build"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "corpus/smoke/$target" \
        --output "$artifact_root/$target-build" \
        --target "$target" \
        --artifact "$artifact_root/$target-build.json" \
        -- -O2 -mfpu=fpv4-sp-d16 -mfloat-abi=hard start.S main.c \
        -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -o /workspace/out/smoke.elf
}

build_arm_m3()
{
    target=$1
    toolchain=$2
    mkdir -p "$artifact_root/$target-build"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "corpus/smoke/$target" \
        --output "$artifact_root/$target-build" \
        --target "$target" \
        --artifact "$artifact_root/$target-build.json" \
        -- -O2 start.S main.c \
        -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -o /workspace/out/smoke.elf
}

build_arm_m7()
{
    target=$1
    toolchain=$2
    mkdir -p "$artifact_root/$target-build"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "corpus/smoke/$target" \
        --output "$artifact_root/$target-build" \
        --target "$target" \
        --artifact "$artifact_root/$target-build.json" \
        -- -O2 -mfpu=fpv5-d16 -mfloat-abi=hard start.S main.c \
        -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -o /workspace/out/smoke.elf
}

run_twice()
{
    target=$1
    max_instructions=$2
    expected_uart=$3
    for run in a b
    do
        mkdir -p "$artifact_root/$target-run-$run"
        "$remu" run \
            --target "$target" \
            --elf "$artifact_root/$target-build/smoke.elf" \
            --max-instructions "$max_instructions" \
            --pin 3=1@0 \
            --vcd "$artifact_root/$target-run-$run/signals.vcd" \
            --bus-log "$artifact_root/$target-run-$run/bus.json" \
            --result "$artifact_root/$target-run-$run/result.json"
    done
    cmp "$artifact_root/$target-run-a/result.json" "$artifact_root/$target-run-b/result.json"
    cmp "$artifact_root/$target-run-a/signals.vcd" "$artifact_root/$target-run-b/signals.vcd"
    cmp "$artifact_root/$target-run-a/bus.json" "$artifact_root/$target-run-b/bus.json"
    jq -e --arg target "$target" --argjson uart "$expected_uart" \
        '.target == $target and .reason == "Halted" and .exit_code == 0 and .uart == $uart' \
        "$artifact_root/$target-run-a/result.json" >/dev/null
}

build_arm stm32f411re toolchains/arm-gcc-stm32f411re.toml
run_twice stm32f411re 20000 '[83,84,77,51,50,70,52,49,49,10]'

build_arm_m7 stm32h743zi toolchains/arm-gcc-stm32h743zi.toml
run_twice stm32h743zi 20000 '[83,84,77,51,50,72,55,52,51,10]'

build_arm_m3 stm32f103c8 toolchains/arm-gcc-stm32f103c8.toml
run_twice stm32f103c8 20000 '[83,84,77,51,50,70,49,48,51,10]'

build_arm nrf52840 toolchains/arm-gcc-nrf52840.toml
run_twice nrf52840 20000 '[78,82,70,53,50,56,52,48,10]'

build_arm atsamd51j19a toolchains/arm-gcc-atsamd51j19a.toml
run_twice atsamd51j19a 20000 '[83,65,77,68,53,49,74,49,57,65,10]'

mkdir -p "$artifact_root/esp32p4-build"
"$remu" corpus build \
    --toolchain toolchains/riscv32-esp-gcc-esp32p4.toml \
    --source corpus/smoke/esp32p4 \
    --output "$artifact_root/esp32p4-build" \
    --target esp32p4 \
    --artifact "$artifact_root/esp32p4-build.json" \
    -- -O2 start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map \
    -lgcc -o /workspace/out/smoke.elf
run_twice esp32p4 20000 '[69,83,80,51,50,80,52,10]'

scripts/summarize-new-targets.py \
    --root "$artifact_root" \
    --output qualification/new-targets/results.json \
    --source scripts/qualify-new-targets.sh \
    --source scripts/summarize-new-targets.py \
    --source qualification/new-targets/manifest.json \
    --source qualification/new-targets/registers.md \
    --source crates/remu-machines/src/target.rs \
    --source crates/remu-cpu-arm/src/core.rs \
    --source crates/remu-cpu-riscv/src/core.rs \
    --source crates/remu-devices/src/stm32f1.rs \
    --source crates/remu-devices/src/stm32h7_dma.rs \
    --source crates/remu-devices/src/samd51.rs \
    --source crates/remu-devices/src/nrf52840.rs \
    --source crates/remu-devices/src/esp.rs \
    --source crates/remu-machines/src/arm_mcu_maps.rs \
    --source crates/remu-machines/src/arm_mcu.rs \
    --source crates/remu-machines/src/arm_mcu_new_targets.rs \
    --source crates/remu-machines/src/arm_mcu_stm32h7.rs \
    --source crates/remu-machines/src/arm_mcu_support.rs \
    --source crates/remu-machines/src/arm_mcu_tests.rs \
    --source crates/remu-machines/src/riscv.rs \
    --source corpus/smoke/stm32f411re/start.S \
    --source corpus/smoke/stm32f411re/main.c \
    --source corpus/smoke/stm32f411re/link.ld \
    --source corpus/smoke/stm32h743zi/start.S \
    --source corpus/smoke/stm32h743zi/main.c \
    --source corpus/smoke/stm32h743zi/link.ld \
    --source corpus/smoke/stm32f103c8/start.S \
    --source corpus/smoke/stm32f103c8/main.c \
    --source corpus/smoke/stm32f103c8/link.ld \
    --source corpus/smoke/nrf52840/start.S \
    --source corpus/smoke/nrf52840/main.c \
    --source corpus/smoke/nrf52840/link.ld \
    --source corpus/smoke/atsamd51j19a/start.S \
    --source corpus/smoke/atsamd51j19a/main.c \
    --source corpus/smoke/atsamd51j19a/link.ld \
    --source corpus/smoke/esp32p4/start.S \
    --source corpus/smoke/esp32p4/main.c \
    --source corpus/smoke/esp32p4/link.ld

echo "new-target qualification passed: qualification/new-targets/results.json"
