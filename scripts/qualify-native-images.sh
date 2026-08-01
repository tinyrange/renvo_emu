#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
export PATH=/home/joshua/.cargo/bin:$PATH

artifact_root=${RENVO_NATIVE_ARTIFACT_ROOT:-.renvo/qualification/native-images}
firmware_cache=${RENVO_FIRMWARE_CACHE:-.renvo/firmware/micropython-v1.28.0}
cross_image=sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736
avr_image=sha256:90c1a3cd4d9691b3c902365fb4e3717cc7d1bc155846afe8a759da1de4fb2f8c
msp_image=sha256:c2da13329a24c10f764d480b9aeef07d31cc20f77c2040dfedd2cfe9942dbeb2
package_image=sha256:5aa633e02afc7f2657a5ddc76bdd1ba0f720545e8d6b8690eeca045c29496e09

for image in "$cross_image" "$avr_image" "$msp_image" "$package_image"
do
    docker image inspect "$image" >/dev/null
done
test -s "$firmware_cache/M5STACK_NANOC6-20260406-v1.28.0.bin"
test -s "$firmware_cache/M5STACK_ATOMS3_LITE-20260406-v1.28.0.bin"

cargo build -q -p renvo-cli
renvo=target/debug/renvo
mkdir -p "$artifact_root/build" "$artifact_root/images" "$artifact_root/run" \
    "$artifact_root/rejections"

build_case()
{
    id=$1
    toolchain=$2
    source=$3
    target=$4
    shift 4
    output="$artifact_root/build/$id"
    mkdir -p "$output"
    "$renvo" corpus build \
        --toolchain "$toolchain" \
        --source "$source" \
        --output "$output" \
        --target "$target" \
        --artifact "$artifact_root/build/$id.json" \
        -- "$@"
}

docker_objcopy()
{
    image=$1
    program=$2
    build_id=$3
    format=$4
    extension=$5
    build_path=$(CDPATH= cd -- "$artifact_root/build/$build_id" && pwd)
    docker run --rm --network=none --pull=never \
        --entrypoint "$program" \
        --volume "$build_path:/work" \
        "$image" -O "$format" /work/probe.elf "/work/probe.$extension"
}

build_case wch toolchains/riscv-gcc-rv32ec.toml corpus/native_equivalence/riscv ch32v003 \
    -DSTACK_TOP=0x20000800 \
    -DGPIO_CONFIG_ADDRESS=0x40011000 -DGPIO_CONFIG_VALUE=0x10 \
    -DGPIO_SET_ADDRESS=0x40011010 -DGPIO_SET_VALUE=0x2 \
    start.S -Wl,-T,link-wch.ld -o /workspace/out/probe.elf

build_case rp2040 toolchains/arm-gcc-cortex-m0plus.toml corpus/native_equivalence/arm rp2040 \
    -DRP2040_BOOT2 -DSTACK_TOP=0x20042000 \
    -DGPIO_CONFIG_ADDRESS=0xd0000024 -DGPIO_CONFIG_VALUE=0x02000000 \
    -DGPIO_SET_ADDRESS=0xd0000014 -DGPIO_SET_VALUE=0x02000000 \
    start.S -Wl,-T,link-rp2040.ld -o /workspace/out/probe.elf

build_case rp2350-arm toolchains/arm-gcc-cortex-m33.toml corpus/native_equivalence/arm rp2350 \
    -DSTACK_TOP=0x20082000 \
    -DGPIO_CONFIG_ADDRESS=0xd0000024 -DGPIO_CONFIG_VALUE=0x02000000 \
    -DGPIO_SET_ADDRESS=0xd0000014 -DGPIO_SET_VALUE=0x02000000 \
    start.S -Wl,-T,link-rp2350.ld -o /workspace/out/probe.elf

build_case rp2350-riscv toolchains/riscv-gcc-rv32imac.toml \
    corpus/native_equivalence/riscv rp2350 \
    -DRP2350_IMAGE -DSTACK_TOP=0x20082000 \
    -DGPIO_CONFIG_ADDRESS=0xd0000024 -DGPIO_CONFIG_VALUE=0x02000000 \
    -DGPIO_SET_ADDRESS=0xd0000014 -DGPIO_SET_VALUE=0x02000000 \
    start.S -Wl,-T,link-rp2350.ld -o /workspace/out/probe.elf

build_case esp32c6 toolchains/riscv-gcc-rv32imac.toml corpus/native_equivalence/riscv esp32c6 \
    -DSTACK_TOP=0x40880000 \
    -DGPIO_CONFIG_ADDRESS=0x60091024 -DGPIO_CONFIG_VALUE=0x4 \
    -DGPIO_SET_ADDRESS=0x60091008 -DGPIO_SET_VALUE=0x4 \
    start.S -Wl,-T,link-esp32c6.ld -o /workspace/out/probe.elf

build_case esp32s3 toolchains/xtensa-esp-gcc-esp32s3.toml \
    corpus/native_equivalence/xtensa esp32s3 \
    main.c -Wl,-T,link.ld -o /workspace/out/probe.elf

build_case atsamd21e18 toolchains/arm-gcc-atsamd21e18.toml \
    corpus/native_equivalence/arm atsamd21e18 \
    -DSTACK_TOP=0x20008000 \
    -DGPIO_CONFIG_ADDRESS=0x41004408 -DGPIO_CONFIG_VALUE=0x80 \
    -DGPIO_SET_ADDRESS=0x41004418 -DGPIO_SET_VALUE=0x80 \
    start.S -Wl,-T,link-samd21.ld -o /workspace/out/probe.elf

build_case stm32l432kc toolchains/arm-gcc-stm32l432kc.toml \
    corpus/native_equivalence/arm stm32l432kc \
    -DSTACK_TOP=0x20010000 \
    -DGPIO_CONFIG_ADDRESS=0x48000000 -DGPIO_CONFIG_VALUE=0x400 \
    -DGPIO_SET_ADDRESS=0x48000018 -DGPIO_SET_VALUE=0x20 \
    start.S -Wl,-T,link-stm32l4.ld -o /workspace/out/probe.elf

build_case r7fa4m1ab3cfm toolchains/arm-gcc-r7fa4m1ab3cfm.toml \
    corpus/native_equivalence/arm r7fa4m1ab3cfm \
    -DSTACK_TOP=0x20008000 \
    -DGPIO_CONFIG_ADDRESS=0x4004086c -DGPIO_CONFIG_VALUE=0x5 \
    -DGPIO_SET_ADDRESS=0x40040028 -DGPIO_SET_VALUE=0x800 \
    start.S -Wl,-T,link-samd21.ld -o /workspace/out/probe.elf

build_case atmega328pb toolchains/avr-gcc-atmega328pb.toml corpus/smoke/atmega328pb \
    atmega328pb -O2 main.c -Wl,-Map,/workspace/out/probe.map \
    -o /workspace/out/probe.elf
build_case msp430fr2433 toolchains/msp430-gcc-msp430fr2433.toml \
    corpus/smoke/msp430fr2433 msp430fr2433 \
    -O2 main.c -Wl,-Map,/workspace/out/probe.map -o /workspace/out/probe.elf
build_case pic16f15376 toolchains/xc8-pic16f15376.toml corpus/smoke/pic16f15376 \
    pic16f15376 -O2 main.c -Wl,-Map=/workspace/out/probe.map \
    -o /workspace/out/probe.elf
build_case efm8bb52f32g toolchains/sdcc-mcs51-efm8bb52.toml \
    corpus/smoke/efm8bb52f32g efm8bb52f32g \
    --opt-code-speed main.c -o /workspace/out/probe.ihx

docker_objcopy "$cross_image" riscv64-unknown-elf-objcopy wch binary bin
for id in rp2040 rp2350-arm atsamd21e18 stm32l432kc r7fa4m1ab3cfm
do
    docker_objcopy "$cross_image" arm-none-eabi-objcopy "$id" binary bin
done
docker_objcopy "$cross_image" riscv64-unknown-elf-objcopy rp2350-riscv binary bin
docker_objcopy "$avr_image" avr-objcopy atmega328pb ihex hex
docker_objcopy "$msp_image" msp430-elf-objcopy msp430fr2433 ihex hex

package_root=$(CDPATH= cd -- "$artifact_root" && pwd)
package_tool=$(CDPATH= cd -- scripts && pwd)/package-native.py
package_python=/opt/esptool-5.3/bin/python
docker run --rm --network=none --pull=never --entrypoint "$package_python" \
    --volume "$package_tool:/tool.py:ro" --volume "$package_root:/work" \
    "$package_image" /tool.py uf2 --input /work/build/rp2040/probe.bin \
    --output /work/images/rp2040.uf2 --address 0x10000000 --family 0xe48bff56
docker run --rm --network=none --pull=never --entrypoint "$package_python" \
    --volume "$package_tool:/tool.py:ro" --volume "$package_root:/work" \
    "$package_image" /tool.py uf2 --input /work/build/rp2350-arm/probe.bin \
    --output /work/images/rp2350-arm.uf2 --address 0x10000000 --family 0xe48bff59
docker run --rm --network=none --pull=never --entrypoint "$package_python" \
    --volume "$package_tool:/tool.py:ro" --volume "$package_root:/work" \
    "$package_image" /tool.py uf2 --input /work/build/rp2350-riscv/probe.bin \
    --output /work/images/rp2350-riscv.uf2 --address 0x10000000 --family 0xe48bff5a

for item in "esp32c6 esp32c6" "esp32s3 esp32s3"
do
    set -- $item
    build_path=$(CDPATH= cd -- "$artifact_root/build/$1" && pwd)
    docker run --rm --network=none --pull=never \
        --entrypoint /opt/esptool-5.3/bin/esptool --volume "$build_path:/work" \
        "$package_image" --chip "$2" elf2image \
        --output /work/probe.bin /work/probe.elf
done

firmware_path=$(CDPATH= cd -- "$firmware_cache" && pwd)
docker run --rm --network=none --pull=never --entrypoint "$package_python" \
    --volume "$package_tool:/tool.py:ro" --volume "$package_root:/work" \
    --volume "$firmware_path:/firmware:ro" "$package_image" /tool.py overlay \
    --base /firmware/M5STACK_NANOC6-20260406-v1.28.0.bin \
    --application /work/build/esp32c6/probe.bin \
    --output /work/images/esp32c6.bin --offset 0x10000
docker run --rm --network=none --pull=never --entrypoint "$package_python" \
    --volume "$package_tool:/tool.py:ro" --volume "$package_root:/work" \
    --volume "$firmware_path:/firmware:ro" "$package_image" /tool.py overlay \
    --base /firmware/M5STACK_ATOMS3_LITE-20260406-v1.28.0.bin \
    --application /work/build/esp32s3/probe.bin \
    --output /work/images/esp32s3.bin --offset 0x10000

run_pair()
{
    id=$1
    target=$2
    direct_flag=$3
    direct_image=$4
    native_image=$5
    shift 5
    output="$artifact_root/run/$id"
    mkdir -p "$output"
    native_cpu=
    if [ "$id" = rp2350-riscv ]
    then
        native_cpu=--cpu=riscv
    fi
    "$renvo" run --target "$target" "$direct_flag" "$direct_image" \
        --max-instructions 200000 --vcd "$output/direct.vcd" \
        --result "$output/direct.json" "$@"
    "$renvo" firmware boot --target "$target" --image "$native_image" \
        --max-instructions 200000 --vcd "$output/native.vcd" \
        --result "$output/native.json" $native_cpu "$@"
}

run_pair ch32v003 ch32v003 --elf "$artifact_root/build/wch/probe.elf" \
    "$artifact_root/build/wch/probe.bin"
run_pair ch32v006 ch32v006 --elf "$artifact_root/build/wch/probe.elf" \
    "$artifact_root/build/wch/probe.bin"
run_pair rp2040 rp2040 --elf "$artifact_root/build/rp2040/probe.elf" \
    "$artifact_root/images/rp2040.uf2"
run_pair rp2350-arm rp2350 --elf "$artifact_root/build/rp2350-arm/probe.elf" \
    "$artifact_root/images/rp2350-arm.uf2"
run_pair rp2350-riscv rp2350 --elf "$artifact_root/build/rp2350-riscv/probe.elf" \
    "$artifact_root/images/rp2350-riscv.uf2"
run_pair esp32s3 esp32s3 --elf "$artifact_root/build/esp32s3/probe.elf" \
    "$artifact_root/images/esp32s3.bin"
run_pair esp32c6 esp32c6 --elf "$artifact_root/build/esp32c6/probe.elf" \
    "$artifact_root/images/esp32c6.bin"
run_pair atsamd21e18 atsamd21e18 --elf "$artifact_root/build/atsamd21e18/probe.elf" \
    "$artifact_root/build/atsamd21e18/probe.bin"
run_pair stm32l432kc stm32l432kc --elf "$artifact_root/build/stm32l432kc/probe.elf" \
    "$artifact_root/build/stm32l432kc/probe.bin"
run_pair r7fa4m1ab3cfm r7fa4m1ab3cfm --elf \
    "$artifact_root/build/r7fa4m1ab3cfm/probe.elf" \
    "$artifact_root/build/r7fa4m1ab3cfm/probe.bin"
run_pair atmega328pb atmega328pb --elf "$artifact_root/build/atmega328pb/probe.elf" \
    "$artifact_root/build/atmega328pb/probe.hex" --pin 1=0@0 --pin 1=1@2000
run_pair msp430fr2433 msp430fr2433 --elf \
    "$artifact_root/build/msp430fr2433/probe.elf" \
    "$artifact_root/build/msp430fr2433/probe.hex" --pin 1=0@0 --pin 1=1@2000
run_pair pic16f15376 pic16f15376 --hex \
    "$artifact_root/build/pic16f15376/probe.hex" \
    "$artifact_root/build/pic16f15376/probe.hex" --pin 1=1@0
run_pair efm8bb52f32g efm8bb52f32g --hex \
    "$artifact_root/build/efm8bb52f32g/probe.ihx" \
    "$artifact_root/build/efm8bb52f32g/probe.ihx" --pin 1=1@0

expect_failure()
{
    id=$1
    shift
    if "$@" >"$artifact_root/rejections/$id.log" 2>&1
    then
        echo "expected native-image rejection $id succeeded" >&2
        exit 1
    fi
}

expect_failure wrong-rp-family "$renvo" firmware boot --target rp2350 \
    --image "$artifact_root/images/rp2040.uf2" --max-instructions 1
expect_failure wrong-esp-chip "$renvo" firmware boot --target esp32c6 \
    --image "$artifact_root/images/esp32s3.bin" --max-instructions 1
expect_failure raw-format-mismatch "$renvo" firmware boot --target ch32v003 \
    --image "$artifact_root/build/wch/probe.bin" --format uf2 --max-instructions 1
expect_failure wrong-hex-target "$renvo" firmware boot --target pic16f15376 \
    --image "$artifact_root/build/efm8bb52f32g/probe.ihx" --max-instructions 1

scripts/summarize-native-images.py \
    --root "$artifact_root" \
    --output qualification/native-images.json \
    --source scripts/qualify-native-images.sh \
    --source scripts/package-native.py \
    --source scripts/summarize-native-images.py \
    --source corpus/native_equivalence/arm/start.S \
    --source corpus/native_equivalence/arm/link-rp2040.ld \
    --source corpus/native_equivalence/arm/link-rp2350.ld \
    --source corpus/native_equivalence/arm/link-samd21.ld \
    --source corpus/native_equivalence/arm/link-stm32l4.ld \
    --source corpus/native_equivalence/riscv/start.S \
    --source corpus/native_equivalence/riscv/link-wch.ld \
    --source corpus/native_equivalence/riscv/link-rp2350.ld \
    --source corpus/native_equivalence/riscv/link-esp32c6.ld \
    --source corpus/native_equivalence/xtensa/main.c \
    --source corpus/native_equivalence/xtensa/link.ld
