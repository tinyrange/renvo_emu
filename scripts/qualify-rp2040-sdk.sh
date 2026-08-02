#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/rp2040-sdk
artifact=qualification/rp2040-sdk.json
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
    destination=$1
    mkdir -p "$destination"
    cp -R corpus/vendor/pico/. "$destination"/
}

build()
{
    name=$1
    source=$2
    shift 2
    output="$root/$name/out"
    mkdir -p "$output"
    "$remu" corpus build \
        --toolchain toolchains/arm-gcc-cortex-m0plus.toml \
        --source "$source" \
        --output "$output" \
        --target rp2040 \
        --artifact "$root/$name/build.json" \
        -- "$@" -O2 -I. -Wl,-T,arm-link.ld -o /workspace/out/sample.elf
}

run()
{
    name=$1
    elf=$2
    "$remu" run \
        --target rp2040 \
        --elf "$elf" \
        --max-instructions 100000 \
        --vcd "$root/$name/trace.vcd" \
        --bus-log "$root/$name/bus.json" \
        --result "$root/$name/run.json"
}

commit=c81c855ffdedc825975a40ba357723a71358ddf0
uart_path=uart/hello_uart/hello_uart.c
uart_hash=7fe8455ecd2bb246eba1a2f9a4c018e68b53a2fd9c893b98095151fbc5507872
pwm_path=pwm/hello_pwm/hello_pwm.c
pwm_hash=7015a851cb67d5e792661c16ca9497c5d416bdb852f7a075449e24a14137db9d

uart_source="$root/source-uart"
copy_adapter "$uart_source"
fetch_exact \
    "https://raw.githubusercontent.com/raspberrypi/pico-examples/$commit/$uart_path" \
    "$uart_hash" "$uart_source/hello_uart.c"
build uart "$uart_source" arm-start.S hello_uart.c compat.c
run uart "$root/uart/out/sample.elf"

pwm_source="$root/source-pwm"
copy_adapter "$pwm_source"
fetch_exact \
    "https://raw.githubusercontent.com/raspberrypi/pico-examples/$commit/$pwm_path" \
    "$pwm_hash" "$pwm_source/hello_pwm.c"
build pwm "$pwm_source" arm-start.S hello_pwm.c compat.c
run pwm "$root/pwm/out/sample.elf"

jq -e '.reason == "Halted" and .exit_code == 0 and (.uart | implode == "AB Hello, UART!\r\n")' \
    "$root/uart/run.json" >/dev/null
jq -e 'any(.[]; .kind == "Write" and .address == 1073954816)' \
    "$root/uart/bus.json" >/dev/null
jq -e 'any(.[]; .kind == "Write" and .address == 1073823744) and
       any(.[]; .kind == "Write" and .address == 1073823752)' \
    "$root/uart/bus.json" >/dev/null
jq -e '.reason == "Halted" and .exit_code == 0 and (.uart | length == 0)' \
    "$root/pwm/run.json" >/dev/null
jq -e 'any(.[]; .kind == "Write" and .address == 1073823744) and
       any(.[]; .kind == "Write" and .address == 1073823752) and
       any(.[]; .kind == "Write" and .address == 1074069520 and .value == 3) and
       any(.[]; .kind == "Write" and .address == 1074069516 and .value == 196609) and
       any(.[]; .kind == "Write" and .address == 1074069664 and .value == 1)' \
    "$root/pwm/bus.json" >/dev/null

adapter_sha=$(find corpus/vendor/pico -type f -print | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d ' ' -f 1)
script_sha=$(sha256 scripts/qualify-rp2040-sdk.sh)
uart_build_sha=$(sha256 "$root/uart/build.json")
pwm_build_sha=$(sha256 "$root/pwm/build.json")
uart_run_sha=$(sha256 "$root/uart/run.json")
pwm_run_sha=$(sha256 "$root/pwm/run.json")

jq -n \
    --arg schema remu.rp2040-sdk-qualification.v1 \
    --arg generated_by scripts/qualify-rp2040-sdk.sh \
    --arg source_sha256 "$script_sha" \
    --arg adapter_sha256 "$adapter_sha" \
    --arg repository https://github.com/raspberrypi/pico-examples \
    --arg commit "$commit" \
    --arg uart_path "$uart_path" --arg uart_hash "$uart_hash" \
    --arg pwm_path "$pwm_path" --arg pwm_hash "$pwm_hash" \
    --arg uart_build "$root/uart/build.json" --arg uart_build_sha "$uart_build_sha" \
    --arg pwm_build "$root/pwm/build.json" --arg pwm_build_sha "$pwm_build_sha" \
    --arg uart_run "$root/uart/run.json" --arg uart_run_sha "$uart_run_sha" \
    --arg pwm_run "$root/pwm/run.json" --arg pwm_run_sha "$pwm_run_sha" \
    '{schema: $schema, generated_by: $generated_by, result: "pass",
      source_sha256: $source_sha256, adapter_sha256: $adapter_sha256,
      repository: $repository, commit: $commit,
      cases: [
        {name: "hello_uart", path: $uart_path, source_sha256: $uart_hash,
         build: $uart_build, build_sha256: $uart_build_sha,
         run: $uart_run, run_sha256: $uart_run_sha,
         observables: ["RP2040 UART0 transmit", "GPIO function select", "CRLF conversion"]},
        {name: "hello_pwm", path: $pwm_path, source_sha256: $pwm_hash,
         build: $pwm_build, build_sha256: $pwm_build_sha,
         run: $pwm_run, run_sha256: $pwm_run_sha,
         observables: ["PWM GPIO function select", "slice wrap", "channel compare", "enable mask"]}
      ]}' >"$artifact"

jq -e '.schema == "remu.rp2040-sdk-qualification.v1" and .result == "pass" and (.cases | length == 2)' \
    "$artifact" >/dev/null
echo "pinned RP2040 Pico SDK UART/PWM qualification passed; artifact: $artifact"
