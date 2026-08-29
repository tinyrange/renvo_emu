#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

python3 scripts/validate-oracles.py

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/remu-oracle.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
fixture=qualification/oracles/fixtures/nanoc6-edge-corpus-v1.json
for mutation in observation count reference-path fixture-id
do
    mkdir -p "$tmp_dir/$mutation"
    mutated="$tmp_dir/$mutation/nanoc6-edge-corpus-v1.json"
    cp "$fixture" "$mutated"
    python3 - "$mutated" "$mutation" <<'PY'
import json
import sys

path = sys.argv[1]
mutation = sys.argv[2]
with open(path, encoding="utf-8") as stream:
    fixture = json.load(stream)
if mutation == "observation":
    fixture["observations"]["records"][0]["value_hex"] = "0x00000000"
elif mutation == "count":
    fixture["comparison"]["expected_observations"] = 39
elif mutation == "reference-path":
    fixture["comparison"]["reference"] = "../outside.tsv"
elif mutation == "fixture-id":
    fixture["fixture_id"] = "wrong-fixture-id"
with open(path, "w", encoding="utf-8") as stream:
    json.dump(fixture, stream, indent=2)
    stream.write("\n")
PY

    if python3 scripts/validate-oracles.py --fixture "$mutated" >/dev/null 2>&1
    then
        echo "oracle $mutation mutation unexpectedly passed" >&2
        exit 1
    fi
done

echo "oracle replay and mismatch rejection passed"
