#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
python3 scripts/generate-dashboard.py
jq -e '.schema == "remu.support-dashboard.v1" and .result == "pass" and (.targets | length == 6) and ([.targets[].register_coverage.status] | all(. == "passing")) and ([.targets[].known_gaps | length] | all(. > 0))' qualification/dashboard.json >/dev/null
