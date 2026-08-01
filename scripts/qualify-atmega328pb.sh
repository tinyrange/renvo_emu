#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:90c1a3cd4d9691b3c902365fb4e3717cc7d1bc155846afe8a759da1de4fb2f8c renvo/avr-gcc:local)
artifact_root=${ATMEGA328PB_ARTIFACT_ROOT:-.renvo/qualification/atmega328pb}
toolchain=toolchains/avr-gcc-atmega328pb.toml

docker image inspect "$image" >/dev/null
cargo build -q -p renvo-cli
renvo=target/debug/renvo
mkdir -p "$artifact_root" "$artifact_root/run-a" "$artifact_root/run-b"

for optimization in O0 Os O2; do
    build="$artifact_root/build-$optimization"
    mkdir -p "$build"
    "$renvo" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/atmega328pb \
        --output "$build" \
        --target atmega328pb \
        --artifact "$artifact_root/build-$optimization.json" \
        -- "-$optimization" main.c \
        -Wl,-Map,/workspace/out/smoke.map \
        -o /workspace/out/smoke.elf
done

for optimization in O0 Os; do
    mkdir -p "$artifact_root/run-$optimization"
    "$renvo" run \
        --target atmega328pb \
        --elf "$artifact_root/build-$optimization/smoke.elf" \
        --max-instructions 150000 \
        --pin 1=0@0 --pin 1=1@1000 \
        --vcd "$artifact_root/run-$optimization/signals.vcd" \
        --result "$artifact_root/run-$optimization/result.json"
    jq -e '.target == "atmega328pb" and .reason == "Halted" and .exit_code == 0 and (.uart == [65,86,82,56,45,80,66,10])' \
        "$artifact_root/run-$optimization/result.json" >/dev/null
done

docker run --rm --network=none \
    -v "$repo_root/$artifact_root/build-O2:/workspace/out" \
    "$image" avr-objcopy -O ihex /workspace/out/smoke.elf /workspace/out/smoke.hex
docker run --rm --network=none \
    -v "$repo_root/$artifact_root/build-O2:/workspace/out" \
    "$image" avr-objdump -d -S /workspace/out/smoke.elf >"$artifact_root/build-O2/smoke.disasm"

run_once()
{
    output=$1
    "$renvo" run \
        --target atmega328pb \
        --elf "$artifact_root/build-O2/smoke.elf" \
        --max-instructions 100000 \
        --pin 1=0@0 \
        --pin 1=1@1000 \
        --vcd "$output/signals.vcd" \
        --bus-log "$output/bus.json" \
        --result "$output/result.json"
}

run_once "$artifact_root/run-a"
run_once "$artifact_root/run-b"
cmp "$artifact_root/run-a/result.json" "$artifact_root/run-b/result.json"
cmp "$artifact_root/run-a/signals.vcd" "$artifact_root/run-b/signals.vcd"
jq -e '.target == "atmega328pb" and .reason == "Halted" and .exit_code == 0 and (.uart == [65,86,82,56,45,80,66,10])' \
    "$artifact_root/run-a/result.json" >/dev/null
grep -q '^\$scope module atmega328pb \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module timer0 \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module usart0 \$end$' "$artifact_root/run-a/signals.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/run-a/signals.vcd"
grep -q 'timer0' "$artifact_root/run-a/signals.vcd"
grep -q 'usart0' "$artifact_root/run-a/signals.vcd"
grep -q 'pcint0' "$artifact_root/run-a/signals.vcd"

upstream_root="$repo_root/.renvo/upstream/avr-libc"
upstream_revision=3b40a25de8948396d0055565b791d80fbd02cab7
if [ ! -d "$upstream_root/.git" ]; then
    mkdir -p "$repo_root/.renvo/upstream"
    git clone --filter=blob:none https://github.com/avrdudes/avr-libc.git "$upstream_root"
fi
git -C "$upstream_root" checkout --detach "$upstream_revision" >/dev/null
test "$(git -C "$upstream_root" rev-parse HEAD)" = "$upstream_revision"

official_source="$artifact_root/avr-libc-source"
official_build="$artifact_root/avr-libc-build"
official_run="$artifact_root/avr-libc-run"
mkdir -p "$official_source" "$official_build" "$official_run"
cp qualification/atmega328pb/avr-libc/adapter.c "$official_source/adapter.c"
cp "$upstream_root/doc/examples/stdiodemo/uart.c" "$official_source/uart.c"
cp "$upstream_root/doc/examples/stdiodemo/uart.h" "$official_source/uart.h"
cp "$upstream_root/doc/examples/stdiodemo/defines.h" "$official_source/defines.h"

"$renvo" corpus build \
    --toolchain "$toolchain" \
    --source "$official_source" \
    --output "$official_build" \
    --target atmega328pb \
    --artifact "$artifact_root/avr-libc-build.json" \
    -- -Os -I. adapter.c uart.c -Wl,-Map,/workspace/out/official.map \
    -o /workspace/out/official.elf

"$renvo" run \
    --target atmega328pb \
    --elf "$official_build/official.elf" \
    --max-instructions 100000 \
    --vcd "$official_run/signals.vcd" \
    --result "$official_run/result.json"
jq -e '.target == "atmega328pb" and .reason == "Halted" and .exit_code == 0 and (.uart == [79,70,70,73,67,73,65,76,13,10])' \
    "$official_run/result.json" >/dev/null

echo "ATmega328PB Docker/offline qualification passed: $artifact_root"
