#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:c2da13329a24c10f764d480b9aeef07d31cc20f77c2040dfedd2cfe9942dbeb2 remu/msp430-gcc:local)
artifact_root=${MSP430FR2433_ARTIFACT_ROOT:-.remu/qualification/msp430fr2433}
toolchain=toolchains/msp430-gcc-msp430fr2433.toml

docker image inspect "$image" >/dev/null
cargo build -q -p remu-cli
remu=target/debug/remu
mkdir -p "$artifact_root" "$artifact_root/run-a" "$artifact_root/run-b"

for optimization in O0 Os O2; do
    build="$artifact_root/build-$optimization"
    mkdir -p "$build"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/msp430fr2433 \
        --output "$build" \
        --target msp430fr2433 \
        --artifact "$artifact_root/build-$optimization.json" \
        -- "-$optimization" main.c \
        -Wl,-Map,/workspace/out/smoke.map \
        -o /workspace/out/smoke.elf
done

for optimization in O0 Os; do
    mkdir -p "$artifact_root/run-$optimization"
    "$remu" run \
        --target msp430fr2433 \
        --elf "$artifact_root/build-$optimization/smoke.elf" \
        --max-instructions 200000 \
        --pin 1=0@0 --pin 1=1@2000 \
        --vcd "$artifact_root/run-$optimization/signals.vcd" \
        --result "$artifact_root/run-$optimization/result.json"
    jq -e '.target == "msp430fr2433" and .reason == "Halted" and .exit_code == 0 and (.uart == [77,83,80,52,51,48,88,45,70,82,50,52,51,51,10])' \
        "$artifact_root/run-$optimization/result.json" >/dev/null
done

docker run --rm --network=none \
    -v "$repo_root/$artifact_root/build-O2:/workspace/out" \
    "$image" msp430-elf-objcopy -O ihex \
    /workspace/out/smoke.elf /workspace/out/smoke.hex
docker run --rm --network=none \
    -v "$repo_root/$artifact_root/build-O2:/workspace/out" \
    "$image" msp430-elf-objdump -d -S /workspace/out/smoke.elf \
    >"$artifact_root/build-O2/smoke.disasm"

run_once()
{
    output=$1
    "$remu" run \
        --target msp430fr2433 \
        --elf "$artifact_root/build-O2/smoke.elf" \
        --max-instructions 150000 \
        --pin 1=0@0 \
        --pin 1=1@2000 \
        --vcd "$output/signals.vcd" \
        --bus-log "$output/bus.json" \
        --result "$output/result.json"
}

run_once "$artifact_root/run-a"
run_once "$artifact_root/run-b"
cmp "$artifact_root/run-a/result.json" "$artifact_root/run-b/result.json"
cmp "$artifact_root/run-a/signals.vcd" "$artifact_root/run-b/signals.vcd"
jq -e '.target == "msp430fr2433" and .reason == "Halted" and .exit_code == 0 and (.uart == [77,83,80,52,51,48,88,45,70,82,50,52,51,51,10])' \
    "$artifact_root/run-a/result.json" >/dev/null
grep -q '^\$scope module msp430fr2433 \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module timer_a0 \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module uart0 \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/run-a/signals.vcd"
grep -q 'timer_a0' "$artifact_root/run-a/signals.vcd"
grep -q 'uart0' "$artifact_root/run-a/signals.vcd"
grep -q 'interrupt' "$artifact_root/run-a/signals.vcd"
grep -q 'port1' "$artifact_root/run-a/signals.vcd"

upstream_dir="$repo_root/.remu/upstream/ti-slac700"
archive="$upstream_dir/slac700e.zip"
archive_url=https://dr-download.ti.com/software-development/code-example-or-demo/MD-nwyaC2If08/01.00.00.0E/slac700e.zip
archive_sha=9d8b339b98949afd26a31609d16baed06257eeb06126043ac0a3eda09397e12d
mkdir -p "$upstream_dir"
if [ ! -f "$archive" ]; then
    curl --fail --location --output "$archive" "$archive_url"
fi
echo "$archive_sha  $archive" | sha256sum -c -

official_source="$artifact_root/slac700-source"
mkdir -p "$official_source"
archive_prefix=MSP430FR243x_MSP430FR253x_MSP430FR263x_Code_Examples/C
unzip -j -o -q "$archive" \
    "$archive_prefix/msp430fr243x_P1_01.c" \
    "$archive_prefix/msp430fr243x_ta0_02.c" \
    "$archive_prefix/msp430fr243x_euscia0_uart_03.c" \
    -d "$official_source"
cp qualification/msp430fr2433/slac700/uart_loopback_adapter.c "$official_source/"

build_official()
{
    name=$1
    shift
    output="$artifact_root/slac700-$name-build"
    mkdir -p "$output"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source "$official_source" \
        --output "$output" \
        --target msp430fr2433 \
        --artifact "$artifact_root/slac700-$name-build.json" \
        -- -Os "$@" -Wl,-Map,/workspace/out/official.map \
        -o /workspace/out/official.elf
    docker run --rm --network=none \
        -v "$repo_root/$output:/workspace/out" \
        "$image" msp430-elf-objcopy -O ihex \
        /workspace/out/official.elf /workspace/out/official.hex
    docker run --rm --network=none \
        -v "$repo_root/$output:/workspace/out" \
        "$image" msp430-elf-objdump -d -S /workspace/out/official.elf \
        >"$output/official.disasm"
}

build_official gpio msp430fr243x_P1_01.c
build_official timer msp430fr243x_ta0_02.c
build_official uart msp430fr243x_euscia0_uart_03.c uart_loopback_adapter.c -Wl,--wrap=main

mkdir -p "$artifact_root/slac700-gpio-run" "$artifact_root/slac700-timer-run" "$artifact_root/slac700-uart-run"
"$remu" run \
    --target msp430fr2433 \
    --elf "$artifact_root/slac700-gpio-build/official.elf" \
    --max-instructions 20000 \
    --pin 3=0@0 --pin 3=1@1000 \
    --stop-signal board.msp430fr2433.port1.pin0=rising \
    --vcd "$artifact_root/slac700-gpio-run/signals.vcd" \
    --result "$artifact_root/slac700-gpio-run/result.json"
jq -e '.reason == {"Signal":"board.msp430fr2433.port1.pin0"}' \
    "$artifact_root/slac700-gpio-run/result.json" >/dev/null

"$remu" run \
    --target msp430fr2433 \
    --elf "$artifact_root/slac700-timer-build/official.elf" \
    --max-instructions 100000 \
    --stop-signal board.msp430fr2433.port1.pin0=falling \
    --vcd "$artifact_root/slac700-timer-run/signals.vcd" \
    --result "$artifact_root/slac700-timer-run/result.json"
jq -e '.reason == {"Signal":"board.msp430fr2433.port1.pin0"}' \
    "$artifact_root/slac700-timer-run/result.json" >/dev/null
grep -q 'timer_a0' "$artifact_root/slac700-timer-run/signals.vcd"

"$remu" run \
    --target msp430fr2433 \
    --elf "$artifact_root/slac700-uart-build/official.elf" \
    --max-instructions 20000 \
    --vcd "$artifact_root/slac700-uart-run/signals.vcd" \
    --result "$artifact_root/slac700-uart-run/result.json"
jq -e '.reason == "InstructionLimit" and (.uart | length) >= 4 and .uart[0:4] == [1,2,3,4]' \
    "$artifact_root/slac700-uart-run/result.json" >/dev/null

find "$artifact_root" -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum >"$artifact_root/SHA256SUMS"

echo "MSP430FR2433 Docker/offline qualification passed: $artifact_root"
