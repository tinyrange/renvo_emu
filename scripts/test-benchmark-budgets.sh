#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/remu-budget.XXXXXX")
trap 'find "$temp_dir" -type f -delete; rmdir "$temp_dir"' EXIT HUP INT TERM

python3 - "$temp_dir" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
common = {
    "id": "rp2040-arm",
    "variant": "performance",
    "iterations": 250,
    "abstract_ticks": 1000,
    "machine_instructions": 1000,
    "iterations_per_million_actions": 250000.0,
    "peak_rss_bytes": 100,
    "result_artifact_bytes": 10,
}
current = {
    "schema": "renvo.coremark-qualification.v1",
    "host": {"model": "budget-test"},
    "runs": [{**common, "host_elapsed_seconds": 1.0, "host_iterations_per_second": 250.0}],
    "observability_modes": [],
}
baseline = {
    **current,
    "runs": [{**common, "host_elapsed_seconds": 0.8, "host_iterations_per_second": 312.5,
               "peak_rss_bytes": 80, "result_artifact_bytes": 8}],
}
(root / "current.json").write_text(json.dumps(current))
(root / "baseline.json").write_text(json.dumps(baseline))
PY

python3 "$repo_root/scripts/check-benchmark-budgets.py" \
    "$temp_dir/current.json" --baseline "$temp_dir/baseline.json"

python3 - "$temp_dir/current.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text())
value["host"]["model"] = "different-host"
path.write_text(json.dumps(value))
PY

if python3 "$repo_root/scripts/check-benchmark-budgets.py" \
    "$temp_dir/current.json" --baseline "$temp_dir/baseline.json" >/dev/null 2>&1
then
    echo "cross-host benchmark comparison unexpectedly passed" >&2
    exit 1
fi

python3 - "$temp_dir/current.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
value = json.loads(path.read_text())
value["host"]["model"] = "budget-test"
value["runs"][0]["host_elapsed_seconds"] = 2.0
path.write_text(json.dumps(value))
PY

if python3 "$repo_root/scripts/check-benchmark-budgets.py" \
    "$temp_dir/current.json" --baseline "$temp_dir/baseline.json" >/dev/null 2>&1
then
    echo "benchmark wall-time regression unexpectedly passed" >&2
    exit 1
fi

echo "benchmark budget comparison passed"
