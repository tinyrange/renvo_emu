#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/vendor-samples
artifact=qualification/vendor-samples.json
rm -rf "$root"
mkdir -p "$root"

sha256()
{
    sha256sum "$1" | cut -d ' ' -f 1
}

fetch_exact()
{
    url=$1
    expected=$2
    output=$3
    curl --fail --location --silent --show-error "$url" --output "$output"
    actual=$(sha256 "$output")
    if [ "$actual" != "$expected" ]; then
        echo "upstream sample hash mismatch for $url: expected $expected, got $actual" >&2
        exit 1
    fi
}

copy_adapter()
{
    adapter=$1
    destination=$2
    mkdir -p "$destination"
    cp -R "$adapter"/. "$destination"/
}

build()
{
    name=$1
    toolchain=$2
    source=$3
    target=$4
    shift 4
    output="$root/$name/out"
    mkdir -p "$output"
    "$remu" corpus build --toolchain "$toolchain" --source "$source" \
        --output "$output" --target "$target" \
        --artifact "$root/$name/build.json" -- "$@" -o /workspace/out/sample.elf
}

run()
{
    name=$1
    target=$2
    elf=$3
    mkdir -p "$root/$name"
    "$remu" run --target "$target" --elf "$elf" --max-instructions 1000000 \
        --vcd "$root/$name/trace.vcd" --bus-log "$root/$name/bus.json" \
        --result "$root/$name/run.json"
    jq -e '.exit_code == 0 and .reason == "Halted"' "$root/$name/run.json" >/dev/null
}

wch_commit=2ac68039c911e0313e2625811b631a8e9e55b9c0
wch_path=EVT/EXAM/GPIO/GPIO_Toggle/User/main.c
wch_hash=b40b009fc1f744b56d4d760a5c3baafd17dfbb621f66f9045cb35131e99d4c0e
wch_source="$root/source-wch"
copy_adapter corpus/vendor/wch "$wch_source"
fetch_exact "https://raw.githubusercontent.com/openwch/ch32v003/$wch_commit/$wch_path" "$wch_hash" "$wch_source/main.c"
build wch toolchains/riscv-gcc-rv32ec.toml "$wch_source" ch32v003 \
    -O2 -fno-builtin -I. start.S main.c compat.c -Wl,-T,link.ld
run ch32v003 ch32v003 "$root/wch/out/sample.elf"
run ch32v006 ch32v006 "$root/wch/out/sample.elf"
for name in ch32v003 ch32v006
do
    jq -e 'any(.[]; .kind == "Write" and (.address == 1073812496 or .address == 1073812500))' "$root/$name/bus.json" >/dev/null
done

pico_commit=c81c855ffdedc825975a40ba357723a71358ddf0
pico_path=blink_simple/blink_simple.c
pico_hash=2f99a43b29b7bac62c2f15bc08468916f8c8af854eaf37e99d0af5cf03f50c1a
pico_source="$root/source-pico"
copy_adapter corpus/vendor/pico "$pico_source"
fetch_exact "https://raw.githubusercontent.com/raspberrypi/pico-examples/$pico_commit/$pico_path" "$pico_hash" "$pico_source/blink_simple.c"
build rp2040 toolchains/arm-gcc-cortex-m0plus.toml "$pico_source" rp2040 \
    -O2 -I. arm-start.S blink_simple.c compat.c -Wl,-T,arm-link.ld
build rp2350-arm toolchains/arm-gcc-cortex-m33.toml "$pico_source" rp2350 \
    -O2 -I. arm-start.S blink_simple.c compat.c -Wl,-T,arm-link.ld
build rp2350-riscv toolchains/riscv-gcc-rv32imac.toml "$pico_source" rp2350-hazard3 \
    -O2 -I. riscv-start.S blink_simple.c compat.c -Wl,-T,riscv-link.ld
run rp2040 rp2040 "$root/rp2040/out/sample.elf"
run rp2350-arm rp2350 "$root/rp2350-arm/out/sample.elf"
run rp2350-riscv rp2350 "$root/rp2350-riscv/out/sample.elf"
for name in rp2040 rp2350-arm rp2350-riscv
do
    jq -e 'any(.[]; .kind == "Write" and (.address == 3489660948 or .address == 3489660952))' "$root/$name/bus.json" >/dev/null
done

esp_commit=f992ff36f68a783d786d83178e5f85e9a9c76ead
esp_path=examples/get-started/hello_world/main/hello_world_main.c
esp_hash=b2d1c8573307e010d276d2d0df537f1fe752416f8422e267aa8b1661c1744595
esp_source="$root/source-esp-idf"
copy_adapter corpus/vendor/esp-idf "$esp_source"
fetch_exact "https://raw.githubusercontent.com/espressif/esp-idf/$esp_commit/$esp_path" "$esp_hash" "$esp_source/hello_world_main.c"
build esp32s3 toolchains/xtensa-esp-gcc-esp32s3.toml "$esp_source" esp32s3 \
    -O2 -fno-builtin -I. -DCONFIG_IDF_TARGET=\"esp32s3\" xtensa-start.S hello_world_main.c compat.c -Wl,-T,xtensa-link.ld
build esp32c6 toolchains/riscv-gcc-rv32imac.toml "$esp_source" esp32c6 \
    -O2 -fno-builtin -I. -DCONFIG_IDF_TARGET=\"esp32c6\" riscv-start.S hello_world_main.c compat.c -Wl,-T,riscv-link.ld
run esp32s3 esp32s3 "$root/esp32s3/out/sample.elf"
run esp32c6 esp32c6 "$root/esp32c6/out/sample.elf"
for name in esp32s3 esp32c6
do
    jq -e '.uart | implode | startswith("Hello world!\n")' "$root/$name/run.json" >/dev/null
    jq -e 'any(.[]; .kind == "Write" and .address == 1610612736)' "$root/$name/bus.json" >/dev/null
done

targets='ch32v003 ch32v006 rp2040 rp2350-arm rp2350-riscv esp32s3 esp32c6'
target_json=
for name in $targets
do
    case "$name" in
        ch32v003|ch32v006) corpus=wch-evt; sample_hash=$wch_hash; build_case=wch ;;
        rp2040) corpus=pico-sdk; sample_hash=$pico_hash; build_case=rp2040 ;;
        rp2350-arm) corpus=pico-sdk; sample_hash=$pico_hash; build_case=rp2350-arm ;;
        rp2350-riscv) corpus=pico-sdk; sample_hash=$pico_hash; build_case=rp2350-riscv ;;
        esp32s3|esp32c6) corpus=esp-idf; sample_hash=$esp_hash; build_case=$name ;;
    esac
    item=$(jq -n --arg target "$name" --arg corpus "$corpus" --arg sample_sha256 "$sample_hash" \
        --arg build "$root/$build_case/build.json" --arg build_sha256 "$(sha256 "$root/$build_case/build.json")" \
        --arg run "$root/$name/run.json" \
        --arg run_sha256 "$(sha256 "$root/$name/run.json")" \
        '{target: $target, corpus: $corpus, sample_sha256: $sample_sha256,
          result: "pass", build: $build, build_sha256: $build_sha256,
          run: $run, run_sha256: $run_sha256}')
    target_json=${target_json}${target_json:+,}${item}
done

adapter_sha=$(find corpus/vendor -type f -print | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d ' ' -f 1)
source_sha=$(sha256sum \
    scripts/qualify-vendor-samples.sh \
    toolchains/riscv-gcc-rv32ec.toml \
    toolchains/riscv-gcc-rv32imac.toml \
    toolchains/arm-gcc-cortex-m0plus.toml \
    toolchains/arm-gcc-cortex-m33.toml \
    toolchains/xtensa-esp-gcc-esp32s3.toml | sha256sum | cut -d ' ' -f 1)

jq -n \
    --arg schema remu.vendor-sample-qualification.v1 \
    --arg generated_by scripts/qualify-vendor-samples.sh \
    --arg source_sha256 "$source_sha" \
    --arg adapter_sha256 "$adapter_sha" \
    --arg wch_commit "$wch_commit" --arg wch_path "$wch_path" --arg wch_hash "$wch_hash" \
    --arg pico_commit "$pico_commit" --arg pico_path "$pico_path" --arg pico_hash "$pico_hash" \
    --arg esp_commit "$esp_commit" --arg esp_path "$esp_path" --arg esp_hash "$esp_hash" \
    --argjson targets "[$target_json]" \
    '{schema: $schema, generated_by: $generated_by, source_sha256: $source_sha256,
      result: "pass", adapter_sha256: $adapter_sha256,
      sources: [
        {corpus: "wch-evt", repository: "https://github.com/openwch/ch32v003", commit: $wch_commit,
         path: $wch_path, sha256: $wch_hash, source_treatment: "downloaded and compiled unmodified",
         licence: "vendor source-file notice; fetched on demand and not redistributed"},
        {corpus: "pico-sdk", repository: "https://github.com/raspberrypi/pico-examples", commit: $pico_commit,
         path: $pico_path, sha256: $pico_hash, source_treatment: "downloaded and compiled unmodified",
         licence: "BSD-3-Clause"},
        {corpus: "esp-idf", repository: "https://github.com/espressif/esp-idf", commit: $esp_commit,
         path: $esp_path, sha256: $esp_hash, source_treatment: "downloaded and compiled unmodified",
         licence: "CC0-1.0 (sample file); ESP-IDF repository Apache-2.0"}],
      adapter_boundary: "Tracked startup and SDK compatibility code supplies native Renvo Emulator MMIO; upstream sample source is byte-exact and receives no patch.",
      targets: $targets}' >"$artifact"

jq -e '.result == "pass" and (.sources | length == 3) and (.targets | length == 7) and ([.targets[].result] | all(. == "pass"))' "$artifact" >/dev/null
echo "unmodified WCH EVT, Pico SDK, and ESP-IDF samples passed; artifact: $artifact"
