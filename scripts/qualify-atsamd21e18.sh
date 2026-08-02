#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
. scripts/lib/toolchain-images.sh

image=$(resolve_toolchain_image sha256:8f78d0ea26f75e5b44c2ad88175202f66f1e7054c6ec695c72d26948a48ba736 remu/cross-gcc:local)
clang_image=$(resolve_toolchain_image sha256:e31d07e59ec7eb7f05396e787db55127783b8a21bc2e907ebcaef6534d343dac remu/cross-llvm:local)
artifact_root=${ATSAMD21_ARTIFACT_ROOT:-.remu/qualification/atsamd21e18}
toolchain=toolchains/arm-gcc-atsamd21e18.toml
clang_toolchain=toolchains/arm-clang-atsamd21e18.toml

docker image inspect "$image" >/dev/null
docker image inspect "$clang_image" >/dev/null
cargo build -q -p remu-cli
remu=target/debug/remu
mkdir -p "$artifact_root/build" "$artifact_root/run-a" "$artifact_root/run-b"

"$remu" corpus build \
    --toolchain "$toolchain" \
    --source corpus/smoke/atsamd21e18 \
    --output "$artifact_root/build" \
    --target atsamd21e18 \
    --artifact "$artifact_root/build.json" \
    -- -O2 start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map \
    -lgcc -o /workspace/out/smoke.elf

run_once()
{
    output=$1
    "$remu" run \
        --target atsamd21e18 \
        --elf "$artifact_root/build/smoke.elf" \
        --max-instructions 30000 \
        --pin 3=0@0 \
        --pin 3=1@5000 \
        --vcd "$output/gpio.vcd" \
        --bus-log "$output/bus.json" \
        --result "$output/result.json"
}

run_once "$artifact_root/run-a"
run_once "$artifact_root/run-b"
cmp "$artifact_root/run-a/result.json" "$artifact_root/run-b/result.json"
cmp "$artifact_root/run-a/gpio.vcd" "$artifact_root/run-b/gpio.vcd"
jq -e '.target == "atsamd21e18" and .reason == "Halted" and .exit_code == 0 and (.uart == [83,65,77,68,50,49,10])' "$artifact_root/run-a/result.json" >/dev/null
jq -e '[.[] | select(.region == "atsamd21e18.adc")] | length >= 8' "$artifact_root/run-a/bus.json" >/dev/null
grep -q '^\$scope module atsamd21e18 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module porta \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module tc3 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module sercom0 \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^\$scope module interrupt \$end$' "$artifact_root/run-a/gpio.vcd"
grep -q '^0' "$artifact_root/run-a/gpio.vcd"
grep -q '^1' "$artifact_root/run-a/gpio.vcd"

for optimization in O0 Os; do
    mkdir -p "$artifact_root/build-$optimization" "$artifact_root/run-$optimization"
    "$remu" corpus build \
        --toolchain "$toolchain" \
        --source corpus/smoke/atsamd21e18 \
        --output "$artifact_root/build-$optimization" \
        --target atsamd21e18 \
        --artifact "$artifact_root/build-$optimization.json" \
        -- "-$optimization" start.S main.c \
        -Wl,-T,link.ld,-Map,/workspace/out/smoke.map -lgcc \
        -o /workspace/out/smoke.elf
    "$remu" run \
        --target atsamd21e18 \
        --elf "$artifact_root/build-$optimization/smoke.elf" \
        --max-instructions 30000 \
        --pin 3=0@0 --pin 3=1@5000 \
        --vcd "$artifact_root/run-$optimization/gpio.vcd" \
        --result "$artifact_root/run-$optimization/result.json"
    jq -e '.target == "atsamd21e18" and .reason == "Halted" and .exit_code == 0 and (.uart == [83,65,77,68,50,49,10])' \
        "$artifact_root/run-$optimization/result.json" >/dev/null
done

mkdir -p "$artifact_root/clang-build" "$artifact_root/clang-run"
"$remu" corpus build \
    --toolchain "$clang_toolchain" \
    --source corpus/smoke/atsamd21e18 \
    --output "$artifact_root/clang-build" \
    --target atsamd21e18 \
    --artifact "$artifact_root/clang-build.json" \
    -- -O2 start.S main.c -Wl,-T,link.ld,-Map,/workspace/out/smoke.map \
    -lgcc -o /workspace/out/smoke.elf
"$remu" run \
    --target atsamd21e18 \
    --elf "$artifact_root/clang-build/smoke.elf" \
    --max-instructions 30000 \
    --pin 3=0@0 --pin 3=1@5000 \
    --vcd "$artifact_root/clang-run/gpio.vcd" \
    --result "$artifact_root/clang-run/result.json"
jq -e '.target == "atsamd21e18" and .reason == "Halted" and .exit_code == 0 and (.uart == [83,65,77,68,50,49,10])' \
    "$artifact_root/clang-run/result.json" >/dev/null

upstream_root="$repo_root/.remu/upstream/csp_apps_sam_d21_da1"
upstream_revision=9331e79cf2937d2b3166813c6d2886b2481162e3
if [ ! -d "$upstream_root/.git" ]; then
    mkdir -p "$repo_root/.remu/upstream"
    git clone --filter=blob:none --no-checkout \
        https://github.com/Microchip-MPLAB-Harmony/csp_apps_sam_d21_da1.git \
        "$upstream_root"
fi
git -C "$upstream_root" checkout --detach "$upstream_revision" >/dev/null
test "$(git -C "$upstream_root" rev-parse HEAD)" = "$upstream_revision"

harmony_stage="$artifact_root/harmony-port-source"
mkdir -p "$harmony_stage" "$artifact_root/harmony-port-build" "$artifact_root/harmony-port-run"
cp qualification/atsamd21e18/harmony-port/adapter.c "$harmony_stage/adapter.c"
cp qualification/atsamd21e18/harmony-port/definitions.h "$harmony_stage/definitions.h"
cp qualification/atsamd21e18/harmony-port/start.S "$harmony_stage/start.S"
cp qualification/atsamd21e18/harmony-port/link.ld "$harmony_stage/link.ld"
cp qualification/atsamd21e18/harmony-port/stdlib.h "$harmony_stage/stdlib.h"
cp "$upstream_root/apps/port/port_led_on_off_polling/firmware/src/main.c" \
    "$harmony_stage/main.c"

"$remu" corpus build \
    --toolchain "$toolchain" \
    --source "$harmony_stage" \
    --output "$artifact_root/harmony-port-build" \
    --target atsamd21e18 \
    --artifact "$artifact_root/harmony-port-build.json" \
    -- -O2 -I. start.S main.c adapter.c -Wl,-T,link.ld,-Map,/workspace/out/harmony-port.map \
    -o /workspace/out/harmony-port.elf

"$remu" run \
    --target atsamd21e18 \
    --elf "$artifact_root/harmony-port-build/harmony-port.elf" \
    --max-instructions 10000 \
    --pin 3=0@0 \
    --pin 3=1@100 \
    --stop-signal board.atsamd21e18.porta.pin7=rising \
    --vcd "$artifact_root/harmony-port-run/gpio.vcd" \
    --result "$artifact_root/harmony-port-run/result.json"
jq -e '.target == "atsamd21e18" and (.reason.Signal == "board.atsamd21e18.porta.pin7")' \
    "$artifact_root/harmony-port-run/result.json" >/dev/null

echo "ATSAMD21E18 Docker/offline smoke qualification passed: $artifact_root"
