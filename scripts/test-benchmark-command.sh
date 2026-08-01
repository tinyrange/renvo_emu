#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/remu-benchmark.XXXXXX")
trap 'rm -rf "$temp_dir"' EXIT HUP INT TERM

printf 'benchmark fixture\n' > "$temp_dir/artifact.bin"
python3 "$repo_root/scripts/benchmark-command.py" \
    --label smoke \
    --output "$temp_dir/metrics.json" \
    --actions 10 \
    --artifact "$temp_dir/artifact.bin" \
    -- sh -c 'printf smoke'

python3 - "$temp_dir/metrics.json" "$temp_dir/artifact.bin" <<'PY'
import json
import pathlib
import sys

metrics = json.loads(pathlib.Path(sys.argv[1]).read_text())
artifact = pathlib.Path(sys.argv[2])
assert metrics["schema"] == "remu.benchmark-command.v1"
assert metrics["status"] == "pass"
assert metrics["actions"] == 10
assert metrics["actions_per_second"] > 0
assert metrics["peak_rss_bytes"] > 0
assert metrics["artifacts"][0]["bytes"] == artifact.stat().st_size
assert metrics["host"]["model"]
PY

echo "benchmark command measurement passed"
