#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"
export PATH=/home/joshua/.cargo/bin:$PATH

expected_plan_sha=07811884d7c1f46ab87549e9ffc18966925237d437ac48723e396f7edb7e7940
actual_plan_sha=$(sha256sum PLAN.html | cut -d ' ' -f 1)
test "$actual_plan_sha" = "$expected_plan_sha"

artifact_root=${RENVO_EXPANSION_ARTIFACT_ROOT:-.renvo/qualification/expansion}
log_root="$artifact_root/logs"
mkdir -p "$log_root"
start=$(date +%s)

scripts/check-source-layout.sh >"$log_root/source-layout.log" 2>&1
cargo test --workspace --quiet >"$log_root/workspace-tests.log" 2>&1

for spec in \
    toolchains/arm-gcc-atsamd21e18.toml \
    toolchains/arm-clang-atsamd21e18.toml \
    toolchains/arm-gcc-stm32l432kc.toml \
    toolchains/arm-clang-stm32l432kc.toml \
    toolchains/arm-gcc-r7fa4m1ab3cfm.toml \
    toolchains/arm-clang-r7fa4m1ab3cfm.toml \
    toolchains/avr-gcc-atmega328pb.toml \
    toolchains/msp430-gcc-msp430fr2433.toml \
    toolchains/xc8-pic16f15376.toml \
    toolchains/sdcc-mcs51-efm8bb52.toml
do
    image=$(sed -n 's/^image = "\([^"]*\)"/\1/p' "$spec")
    test -n "$image"
    docker image inspect "$image" >/dev/null
done

(scripts/docker-smoke.sh >"$log_root/original-six.log" 2>&1) & p0=$!
(ATSAMD21_ARTIFACT_ROOT="$artifact_root/atsamd21e18" scripts/qualify-atsamd21e18.sh >"$log_root/atsamd21e18.log" 2>&1) & p1=$!
(STM32L432_ARTIFACT_ROOT="$artifact_root/stm32l432kc" scripts/qualify-stm32l432kc.sh >"$log_root/stm32l432kc.log" 2>&1) & p2=$!
(RA4M1_ARTIFACT_ROOT="$artifact_root/r7fa4m1ab3cfm" scripts/qualify-r7fa4m1ab3cfm.sh >"$log_root/r7fa4m1ab3cfm.log" 2>&1) & p3=$!
(ATMEGA328PB_ARTIFACT_ROOT="$artifact_root/atmega328pb" scripts/qualify-atmega328pb.sh >"$log_root/atmega328pb.log" 2>&1) & p4=$!
(MSP430FR2433_ARTIFACT_ROOT="$artifact_root/msp430fr2433" scripts/qualify-msp430fr2433.sh >"$log_root/msp430fr2433.log" 2>&1) & p5=$!
(PIC16F15376_ARTIFACT_ROOT="$artifact_root/pic16f15376" scripts/qualify-pic16f15376.sh >"$log_root/pic16f15376.log" 2>&1) & p6=$!
(EFM8BB52F32G_ARTIFACT_ROOT="$artifact_root/efm8bb52f32g" scripts/qualify-efm8bb52f32g.sh >"$log_root/efm8bb52f32g.log" 2>&1) & p7=$!

failed=0
for job in "$p0" "$p1" "$p2" "$p3" "$p4" "$p5" "$p6" "$p7"
do
    wait "$job" || failed=1
done
if [ "$failed" -ne 0 ]; then
    for log in "$log_root"/*.log
    do
        echo "==> $log" >&2
        tail -n 30 "$log" >&2
    done
    exit 1
fi

elapsed=$(($(date +%s) - start))
scripts/summarize-expansion.py \
    --root "$artifact_root" \
    --evidence evidence/targets.toml \
    --plan PLAN.html \
    --elapsed-seconds "$elapsed" \
    --original-six-log "$log_root/original-six.log" \
    --tests-log "$log_root/workspace-tests.log" \
    --output "$artifact_root/summary.json"

test "$elapsed" -lt 60
echo "comprehensive expansion gate passed in ${elapsed}s; summary: $artifact_root/summary.json"
