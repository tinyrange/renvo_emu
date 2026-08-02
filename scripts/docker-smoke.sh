#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. scripts/lib/toolchain-images.sh

cross_image=$(resolve_toolchain_image \
    sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 \
    remu/cross-gcc:local)
xtensa_image=$(resolve_toolchain_image \
    sha256:e0c54aeaae63f842234ec88f7b5a61b69bfa4d9005ba7490df47328e0dc9892f \
    remu/xtensa-esp-gcc:local)

docker image inspect "$cross_image" >/dev/null
docker image inspect "$xtensa_image" >/dev/null

cargo build -q -p remu-cli
remu=target/debug/remu
artifact_root=.remu/portfolio-smoke
mkdir -p "$artifact_root"

build_case()
{
    name=$1
    toolchain=$2
    source=$3
    target=$4
    shift 4
    mkdir -p "$artifact_root/$name"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "$source" \
        --output "$artifact_root/$name" \
        --target "$target" \
        --artifact "$artifact_root/$name-build.json" \
        -- "$@" -Wl,-T,link.ld -o /workspace/out/smoke.elf
}

run_case()
{
    name=$1
    target=$2
    elf=$3
    "$remu" run \
        --target "$target" \
        --elf "$elf" \
        --max-instructions 10000 \
        --vcd "$artifact_root/$name.vcd" \
        --bus-log "$artifact_root/$name-bus.json" \
        --result "$artifact_root/$name-run.json"
}

build_case wch toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-gpio ch32v003 \
    -O2 start.S main.c
run_case ch32v003 ch32v003 "$artifact_root/wch/smoke.elf"
run_case ch32v006 ch32v006 "$artifact_root/wch/smoke.elf"

build_case wch-uart toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-uart ch32v003 \
    -O2 start.S main.c
run_case ch32v003-uart ch32v003 "$artifact_root/wch-uart/smoke.elf"
run_case ch32v006-uart ch32v006 "$artifact_root/wch-uart/smoke.elf"

build_case wch-timer toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-timer ch32v003 \
    start.S
run_case ch32v003-timer ch32v003 "$artifact_root/wch-timer/smoke.elf"
run_case ch32v006-timer ch32v006 "$artifact_root/wch-timer/smoke.elf"

build_case wch-xw toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-xw ch32v003 \
    start.S
run_case ch32v003-xw ch32v003 "$artifact_root/wch-xw/smoke.elf"
run_case ch32v006-xw ch32v006 "$artifact_root/wch-xw/smoke.elf"

build_case wch-zmmul toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-xw ch32v006 \
    zmmul.S
run_case ch32v006-zmmul ch32v006 "$artifact_root/wch-zmmul/smoke.elf"

build_case riscv-timer toolchains/riscv-gcc-rv32ec.toml corpus/smoke/riscv-timer ch32v003 \
    start.S
run_case riscv-timer ch32v003 "$artifact_root/riscv-timer/smoke.elf"

build_case rp-arm toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/rp-sio rp2040 \
    -O2 start.S main.c
run_case rp2040 rp2040 "$artifact_root/rp-arm/smoke.elf"
run_case rp2350-arm rp2350 "$artifact_root/rp-arm/smoke.elf"

build_case rp2040-uart toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/rp-sio rp2040 \
    -O2 -DUART_BASE=0x40034000u start.S uart.c
run_case rp2040-uart rp2040 "$artifact_root/rp2040-uart/smoke.elf"

build_case rp2350-arm-uart toolchains/arm-gcc-cortex-m33.toml corpus/smoke/rp-sio rp2350 \
    -O2 -DUART_BASE=0x40070000u start.S uart.c
run_case rp2350-arm-uart rp2350 "$artifact_root/rp2350-arm-uart/smoke.elf"

build_case rp-arm-pio toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/rp-sio rp2040 \
    -O2 start.S pio.c
run_case rp2040-pio rp2040 "$artifact_root/rp-arm-pio/smoke.elf"
run_case rp2350-arm-pio rp2350 "$artifact_root/rp-arm-pio/smoke.elf"

build_case arm-timer toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/arm-timer rp2040 \
    start.S
run_case arm-timer rp2040 "$artifact_root/arm-timer/smoke.elf"

build_case rp2040-native-timer toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/rp-native-timer-arm rp2040 \
    -DTIMER_BASE=0x40054000 -DTIMER_INTR_OFFSET=0x34 -DTIMER_INTE_OFFSET=0x38 start.S
run_case rp2040-native-timer rp2040 "$artifact_root/rp2040-native-timer/smoke.elf"

build_case rp2350-arm-native-timer toolchains/arm-gcc-cortex-m33.toml corpus/smoke/rp-native-timer-arm rp2350 \
    -DTIMER_BASE=0x400b0000 -DTIMER_INTR_OFFSET=0x3c -DTIMER_INTE_OFFSET=0x40 start.S
run_case rp2350-arm-native-timer rp2350 "$artifact_root/rp2350-arm-native-timer/smoke.elf"

build_case rp2040-arm-exceptions toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/arm-qualification rp2040 \
    exceptions.S
run_case rp2040-arm-exceptions rp2040 "$artifact_root/rp2040-arm-exceptions/smoke.elf"

build_case rp2350-arm-exceptions toolchains/arm-gcc-cortex-m33.toml corpus/smoke/arm-qualification rp2350 \
    exceptions.S
run_case rp2350-arm-exceptions rp2350 "$artifact_root/rp2350-arm-exceptions/smoke.elf"

build_case rp2350-arm-hardfloat toolchains/arm-gcc-cortex-m33.toml corpus/smoke/arm-qualification rp2350 \
    -O2 -mfpu=fpv5-sp-d16 -mfloat-abi=hard start.S hardfloat.c
run_case rp2350-arm-hardfloat rp2350 "$artifact_root/rp2350-arm-hardfloat/smoke.elf"

build_case rp-riscv toolchains/riscv-gcc-rv32imac.toml corpus/smoke/rp-riscv rp2350 \
    -O2 start.S main.c
run_case rp2350-riscv rp2350 "$artifact_root/rp-riscv/smoke.elf"

mkdir -p "$artifact_root/hazard3"
"$remu" corpus build \
    --toolchain toolchains/riscv-gcc-hazard3.toml \
    --source corpus/smoke/hazard3 \
    --output "$artifact_root/hazard3" \
    --target rp2350-hazard3 \
    --artifact "$artifact_root/hazard3-build.json"
run_case hazard3-extensions rp2350 "$artifact_root/hazard3/smoke.elf"

build_case rp2350-riscv-uart toolchains/riscv-gcc-rv32imac.toml corpus/smoke/rp-riscv rp2350 \
    -O2 start.S uart.c
run_case rp2350-riscv-uart rp2350 "$artifact_root/rp2350-riscv-uart/smoke.elf"

build_case rp2350-riscv-native-timer toolchains/riscv-gcc-rv32imac.toml corpus/smoke/rp-native-timer-riscv rp2350 \
    start.S
run_case rp2350-riscv-native-timer rp2350 "$artifact_root/rp2350-riscv-native-timer/smoke.elf"

build_case rp2350-riscv-pio toolchains/riscv-gcc-rv32imac.toml corpus/smoke/rp-riscv rp2350 \
    -O2 start.S pio.c
run_case rp2350-riscv-pio rp2350 "$artifact_root/rp2350-riscv-pio/smoke.elf"

build_case esp32c6 toolchains/riscv-gcc-rv32imac.toml corpus/smoke/esp32c6 esp32c6 \
    -O2 start.S main.c
run_case esp32c6 esp32c6 "$artifact_root/esp32c6/smoke.elf"

build_case esp32c6-uart toolchains/riscv-gcc-rv32imac.toml corpus/smoke/esp32c6 esp32c6 \
    -O2 start.S uart.c
run_case esp32c6-uart esp32c6 "$artifact_root/esp32c6-uart/smoke.elf"

build_case esp32c6-uarts toolchains/riscv-gcc-rv32imac.toml corpus/smoke/esp32c6 esp32c6 \
    -O2 start.S uart-multi.c
run_case esp32c6-uarts esp32c6 "$artifact_root/esp32c6-uarts/smoke.elf"

build_case esp32c6-privilege toolchains/riscv-gcc-rv32imac.toml corpus/smoke/esp32c6 esp32c6 \
    privilege.S
run_case esp32c6-privilege esp32c6 "$artifact_root/esp32c6-privilege/smoke.elf"

build_case esp32s3 toolchains/xtensa-esp-gcc-esp32s3.toml corpus/smoke/xtensa esp32s3 \
    -O2 main.c
run_case esp32s3 esp32s3 "$artifact_root/esp32s3/smoke.elf"

for optimization in o0 o2 os
do
    case "$optimization" in
        o0) flag=-O0 ;;
        o2) flag=-O2 ;;
        os) flag=-Os ;;
    esac
    build_case "esp32s3-xtensa-$optimization" \
        toolchains/xtensa-esp-gcc-esp32s3.toml \
        corpus/smoke/xtensa-qualification esp32s3 \
        "$flag" exception.S main.c
    run_case "esp32s3-xtensa-$optimization" esp32s3 \
        "$artifact_root/esp32s3-xtensa-$optimization/smoke.elf"
    run_case "esp32s3-xtensa-$optimization-repeat" esp32s3 \
        "$artifact_root/esp32s3-xtensa-$optimization/smoke.elf"
done

build_case esp32s3-uart toolchains/xtensa-esp-gcc-esp32s3.toml corpus/smoke/xtensa esp32s3 \
    -O2 uart.c
run_case esp32s3-uart esp32s3 "$artifact_root/esp32s3-uart/smoke.elf"

build_case stop-riscv-fault toolchains/riscv-gcc-rv32ec.toml corpus/smoke/wch-gpio ch32v003 \
    -O2 start.S fault.c
build_case stop-arm-fault toolchains/arm-gcc-cortex-m0plus.toml corpus/smoke/rp-sio rp2040 \
    -O2 start.S fault.c
build_case stop-xtensa-fault toolchains/xtensa-esp-gcc-esp32s3.toml corpus/smoke/xtensa esp32s3 \
    -O2 fault.c

for result in "$artifact_root"/*-run.json
do
    grep -q '"reason": "Halted"' "$result"
    grep -q '"exit_code":' "$result"
done
grep -q '"events": 1' "$artifact_root/riscv-timer-run.json"
grep -q '"events": 1' "$artifact_root/arm-timer-run.json"
jq -e '.exit_code == 0 and .stats.events == 1' "$artifact_root/ch32v003-timer-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events == 1' "$artifact_root/ch32v006-timer-run.json" >/dev/null
jq -e '.uart | implode == "REMU-WCH\n"' "$artifact_root/ch32v003-uart-run.json" >/dev/null
jq -e '.uart | implode == "REMU-WCH\n"' "$artifact_root/ch32v006-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "REMU-RP\n")' "$artifact_root/rp2040-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "REMU-RP\n")' "$artifact_root/rp2350-arm-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "REMU-RP\n")' "$artifact_root/rp2350-riscv-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "REMU-ESP\n")' "$artifact_root/esp32c6-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "U0\nU1\nLP\n")' "$artifact_root/esp32c6-uarts-run.json" >/dev/null
jq -e '.exit_code == 0 and (.uart | implode == "REMU-ESP\n")' "$artifact_root/esp32s3-uart-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events == 1' "$artifact_root/rp2040-native-timer-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events == 1' "$artifact_root/rp2350-arm-native-timer-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events == 1' "$artifact_root/rp2350-riscv-native-timer-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events >= 8' "$artifact_root/rp2040-pio-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events >= 8' "$artifact_root/rp2350-arm-pio-run.json" >/dev/null
jq -e '.exit_code == 0 and .stats.events >= 8' "$artifact_root/rp2350-riscv-pio-run.json" >/dev/null
grep -q '\$scope module pio0 ' "$artifact_root/rp2040-pio.vcd"
grep -q '\$scope module pio0 ' "$artifact_root/rp2350-arm-pio.vcd"
grep -q '\$scope module pio0 ' "$artifact_root/rp2350-riscv-pio.vcd"

scripts/generate-register-coverage.sh "$artifact_root" qualification/register-coverage
scripts/generate-riscv-cpu-qualification.sh "$artifact_root" qualification/riscv-cpu.json
scripts/generate-arm-cpu-qualification.sh "$artifact_root" qualification/arm-cpu.json
scripts/generate-xtensa-cpu-qualification.sh "$artifact_root" qualification/xtensa-cpu.json
scripts/qualify-stop-conditions.sh
scripts/qualify-rust-abi.sh
scripts/qualify-reduction.sh
scripts/qualify-debug-observability.sh
scripts/qualify-vendor-samples.sh
scripts/qualify-starlark.sh
scripts/generate-dashboard.sh

echo "portfolio Docker smoke passed; artifacts: $artifact_root"
