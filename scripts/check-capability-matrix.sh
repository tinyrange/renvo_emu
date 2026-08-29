#!/usr/bin/env bash
set -euo pipefail

# Generated outputs are checked against the exact clean checkout that produced
# them. Explicitly ignored build products such as target/ do not affect this.
test -z "$(git status --porcelain --untracked-files=all)"
python3 scripts/generate-dashboard.py --check
test -s qualification/capability-matrix.md
echo "capability matrix is current for the checked source tree"
