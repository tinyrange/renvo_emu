#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 remu/cross-gcc:local)
clang_image=$(resolve_toolchain_image sha256:e31d07e59ec7eb7f05396e787db55127783b8a21bc2e907ebcaef6534d343dac remu/cross-llvm:local)
artifact_root=${STM32L432_ARTIFACT_ROOT:-.remu/qualification/stm32l432kc}
toolchain=toolchains/arm-gcc-stm32l432kc.toml
clang_toolchain=toolchains/arm-clang-stm32l432kc.toml

docker image inspect "$image" >/dev/null
docker image inspect "$clang_image" >/dev/null
cargo build -q -p remu-cli
remu=target/debug/remu
mkdir -p "$artifact_root"

qualify_abi()
{
    abi=$1
    optimization=$2
    shift 2
    mkdir -p "$artifact_root/$abi-build" "$artifact_root/$abi-run"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/stm32l432kc \
        --output "$artifact_root/$abi-build" \
        --target stm32l432kc \
        --artifact "$artifact_root/$abi-build.json" \
        -- "$optimization" "$@" start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -lgcc \
        -o /workspace/out/smoke.elf
    "$remu" run \
        --target stm32l432kc \
        --elf "$artifact_root/$abi-build/smoke.elf" \
        --max-instructions 10000 \
        --pin 3=1@0 \
        --vcd "$artifact_root/$abi-run/gpio.vcd" \
        --result "$artifact_root/$abi-run/result.json"
    build_path=$(CDPATH= cd -- "$artifact_root/$abi-build" && pwd)
    docker run --rm --network=none --pull=never --read-only \
        --entrypoint arm-none-eabi-objdump \
        --volume "$build_path:/input:ro" \
        "$image" -d /input/smoke.elf > "$artifact_root/$abi-build/smoke.disassembly"
    jq -e '.target == "stm32l432kc" and .reason == "Halted" and .exit_code == 0 and (.uart == [83,84,77,51,50,76,52,51,50,10])' \
        "$artifact_root/$abi-run/result.json" >/dev/null
}

qualify_abi soft-o0 -O0 -mfloat-abi=soft
qualify_abi soft-os -Os -mfloat-abi=soft
qualify_abi soft -O2 -mfloat-abi=soft
qualify_abi softfp -O2 -mfpu=fpv4-sp-d16 -mfloat-abi=softfp
qualify_abi hard -O2 -mfpu=fpv4-sp-d16 -mfloat-abi=hard
mkdir -p "$artifact_root/hard-repeat"
"$remu" run \
    --target stm32l432kc \
    --elf "$artifact_root/hard-build/smoke.elf" \
    --max-instructions 10000 \
    --pin 3=1@0 \
    --vcd "$artifact_root/hard-repeat/gpio.vcd" \
    --replay "$artifact_root/hard-run/result.json" \
    --result "$artifact_root/hard-repeat/result.json"
cmp "$artifact_root/hard-run/result.json" "$artifact_root/hard-repeat/result.json"
cmp "$artifact_root/hard-run/gpio.vcd" "$artifact_root/hard-repeat/gpio.vcd"
grep -q '^\$scope module gpioa \$end$' "$artifact_root/hard-run/gpio.vcd"
grep -q '^\$scope module tim2 \$end$' "$artifact_root/hard-run/gpio.vcd"
grep -q '^\$scope module usart2 \$end$' "$artifact_root/hard-run/gpio.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/hard-run/gpio.vcd"
grep -q '^1' "$artifact_root/hard-run/gpio.vcd"
grep -q '^0' "$artifact_root/hard-run/gpio.vcd"
grep -q 'vmul\.f32' "$artifact_root/hard-build/smoke.disassembly"
grep -q 'vcvt\.s32\.f32' "$artifact_root/hard-build/smoke.disassembly"

mkdir -p "$artifact_root/clang-build" "$artifact_root/clang-run"
"$remu" corpus build \
    --toolchain "$clang_toolchain" \
    --source corpus/smoke/stm32l432kc \
    --output "$artifact_root/clang-build" \
    --target stm32l432kc \
    --artifact "$artifact_root/clang-build.json" \
    -- -O2 start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map \
    -lgcc -o /workspace/out/smoke.elf
"$remu" run \
    --target stm32l432kc \
    --elf "$artifact_root/clang-build/smoke.elf" \
    --max-instructions 30000 \
    --pin 3=1@0 \
    --vcd "$artifact_root/clang-run/gpio.vcd" \
    --result "$artifact_root/clang-run/result.json"
jq -e '.target == "stm32l432kc" and .reason == "Halted" and .exit_code == 0 and (.uart == [83,84,77,51,50,76,52,51,50,10])' \
    "$artifact_root/clang-run/result.json" >/dev/null

upstream_root="$repo_root/.remu/upstream/STM32CubeL4"
upstream_revision=a6fd67088a77dc546a00cef2aa67ac540abf9c9f
if [ ! -d "$upstream_root/.git" ]; then
    mkdir -p "$repo_root/.remu/upstream"
    git clone --filter=blob:none --no-checkout \
        https://github.com/STMicroelectronics/STM32CubeL4.git "$upstream_root"
fi
git -C "$upstream_root" sparse-checkout init --cone
git -C "$upstream_root" sparse-checkout set Projects/NUCLEO-L432KC
git -C "$upstream_root" checkout --detach "$upstream_revision" >/dev/null
test "$(git -C "$upstream_root" rev-parse HEAD)" = "$upstream_revision"

cube_stage="$artifact_root/cube-gpio-source"
mkdir -p "$cube_stage" "$artifact_root/cube-gpio-build" "$artifact_root/cube-gpio-run"
cp qualification/stm32l432kc/cube-gpio/main.h "$cube_stage/main.h"
cp qualification/stm32l432kc/cube-gpio/stm32l4xx_hal.h "$cube_stage/stm32l4xx_hal.h"
cp qualification/stm32l432kc/cube-gpio/adapter.c "$cube_stage/adapter.c"
cp qualification/stm32l432kc/cube-gpio/start.S "$cube_stage/start.S"
cp qualification/stm32l432kc/cube-gpio/link.ld "$cube_stage/link.ld"
cp "$upstream_root/Projects/NUCLEO-L432KC/Examples/GPIO/GPIO_IOToggle/Src/main.c" \
    "$cube_stage/main.c"

"$remu" corpus build \
    --toolchain "$toolchain" \
    --source "$cube_stage" \
    --output "$artifact_root/cube-gpio-build" \
    --target stm32l432kc \
    --artifact "$artifact_root/cube-gpio-build.json" \
    -- -O2 -I. -mfpu=fpv4-sp-d16 -mfloat-abi=hard start.S main.c adapter.c \
    -Wl,-T,link.ld,-Map,/workspace/out/cube-gpio.map -o /workspace/out/cube-gpio.elf

"$remu" run \
    --target stm32l432kc \
    --elf "$artifact_root/cube-gpio-build/cube-gpio.elf" \
    --max-instructions 10000 \
    --stop-signal board.stm32l432kc.gpiob.pin3=rising \
    --vcd "$artifact_root/cube-gpio-run/gpio.vcd" \
    --result "$artifact_root/cube-gpio-run/result.json"
jq -e '.target == "stm32l432kc" and (.reason.Signal == "board.stm32l432kc.gpiob.pin3")' \
    "$artifact_root/cube-gpio-run/result.json" >/dev/null

echo "STM32L432KC soft/softfp/hard-float qualification passed: $artifact_root"
