#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

python3 scripts/validate-oracles.py

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/remu-oracle.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
fixture=qualification/oracles/fixtures/nanoc6-edge-corpus-v1.json
mutated="$tmp_dir/mutated.json"
cp "$fixture" "$mutated"

python3 - "$mutated" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as stream:
    fixture = json.load(stream)
fixture["observations"]["records"][0]["value_hex"] = "0x00000000"
with open(path, "w", encoding="utf-8") as stream:
    json.dump(fixture, stream, indent=2)
    stream.write("\n")
PY

if python3 scripts/validate-oracles.py --fixture "$mutated" >/dev/null 2>&1
then
    echo "oracle mutation unexpectedly passed" >&2
    exit 1
fi

echo "oracle replay and mismatch rejection passed"
