#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
python3 scripts/generate-dashboard.py "$@"
jq -e '.schema == "remu.support-dashboard.v2" and .result == "pass" and (.targets | length == 6) and (.tier_definitions | length == 3) and (.source_tree_sha256 | length == 64) and ([.targets[].register_coverage.status] | all(. == "passing")) and ([.targets[].known_gaps | length] | all(. > 0)) and ([.targets[].support_tiers | length] | all(. == 3)) and ([.targets[].support_tiers[].evidence[] | .sha256 | length] | all(. == 64))' qualification/dashboard.json >/dev/null
test -s qualification/capability-matrix.md
