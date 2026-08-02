#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
root=.remu/qualification/rp2040-multicore
artifact=qualification/rp2040-multicore.json
rm -rf "$root"
mkdir -p "$root"

sha256()
{
    sha256sum "$1" | cut -d ' ' -f 1
}

source_dir="$root/source"
mkdir -p "$source_dir"
cp -R corpus/vendor/pico-multicore/. "$source_dir"/

commit=c81c855ffdedc825975a40ba357723a71358ddf0
path=multicore/hello_multicore/multicore.c
source_hash=9432de60e2ea64e6f67d892719c580fc97af9918041c5300567b908c70d9a3a0
curl --fail --location --silent --show-error \
    "https://raw.githubusercontent.com/raspberrypi/pico-examples/$commit/$path" \
    --output "$source_dir/multicore.c"
actual_hash=$(sha256 "$source_dir/multicore.c")
if [ "$actual_hash" != "$source_hash" ]; then
    echo "upstream sample hash mismatch: expected $source_hash, got $actual_hash" >&2
    exit 1
fi

output="$root/out"
mkdir -p "$output"
"$remu" corpus build \
    --toolchain toolchains/arm-gcc-cortex-m0plus.toml \
    --source "$source_dir" \
    --output "$output" \
    --target rp2040 \
    --artifact "$root/build.json" \
    -- arm-start.S multicore.c compat.c -O2 -I. -Wl,-T,arm-link.ld -o /workspace/out/multicore.elf

"$remu" run \
    --target rp2040 \
    --elf "$output/multicore.elf" \
    --max-instructions 1000000 \
    --vcd "$root/trace.vcd" \
    --bus-log "$root/bus.json" \
    --result "$root/run.json"

jq -e '.reason == "Halted" and .exit_code == 0' "$root/run.json" >/dev/null
python3 - "$root/run.json" <<'PY'
import json
import sys

result = json.load(open(sys.argv[1], encoding="utf-8"))
text = "".join(chr(value) for value in result["uart"])
prefix = "Hello, multicore!\n"
assert text.startswith(prefix), text

# The two cores share UART0. The functional scheduler advances one core at a
# time, so their character writes are intentionally interleaved. Check each
# expected message as an order-preserving subsequence and require that the
# complete output contains no extra characters.
tail = text[len(prefix):]
expected = ["It's all gone well on core 0!", "Its all gone well on core 1!"]
assert len(tail) == sum(len(message) for message in expected), repr(tail)
for message in expected:
    remaining = iter(tail)
    assert all(character in remaining for character in message), repr(message)
assert "Hmm" not in text, text
PY
jq -e 'any(.[]; .kind == "Write" and .address == 3489661012 and .value == 0) and
       ([.[] | select(.kind == "Write" and .address == 3489661012 and
                      (.value == 0 or .value == 1 or .value == 268435456 or
                       .value == 537133056 or .value > 268435456))] | length >= 6) and
       any(.[]; .kind == "Write" and .address == 3489661012 and .value == 123) and
       any(.[]; .kind == "Read" and .address == 3489661016 and .value == 123)' \
    "$root/bus.json" >/dev/null

adapter_sha=$(find corpus/vendor/pico-multicore -type f -print | LC_ALL=C sort |
    xargs sha256sum | sha256sum | cut -d ' ' -f 1)
script_sha=$(sha256 scripts/qualify-rp2040-multicore.sh)
build_sha=$(sha256 "$root/build.json")
run_sha=$(sha256 "$root/run.json")

jq -n \
    --arg schema remu.rp2040-multicore-qualification.v1 \
    --arg generated_by scripts/qualify-rp2040-multicore.sh \
    --arg source_sha256 "$script_sha" \
    --arg adapter_sha256 "$adapter_sha" \
    --arg repository https://github.com/raspberrypi/pico-examples \
    --arg commit "$commit" --arg path "$path" --arg sample_sha256 "$source_hash" \
    --arg build ".remu/qualification/rp2040-multicore/build.json" --arg build_sha256 "$build_sha" \
    --arg run ".remu/qualification/rp2040-multicore/run.json" --arg run_sha256 "$run_sha" \
    '{schema: $schema, generated_by: $generated_by, result: "pass",
      source_sha256: $source_sha256, adapter_sha256: $adapter_sha256,
      repository: $repository, commit: $commit,
      case: {name: "hello_multicore", path: $path, source_sha256: $sample_sha256,
             build: $build, build_sha256: $build_sha256,
             run: $run, run_sha256: $run_sha256,
             observables: ["core-1 launch handshake", "FIFO push/pop", "core-0/core-1 UART"]}}' \
    >"$artifact"

jq -e '.schema == "remu.rp2040-multicore-qualification.v1" and .result == "pass"' \
    "$artifact" >/dev/null
echo "pinned RP2040 Pico SDK multicore qualification passed; artifact: $artifact"
