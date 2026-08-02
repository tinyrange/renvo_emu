#!/usr/bin/env bash
set -euo pipefail

# Generated outputs are checked against the current source-tree digest. This
# is deliberately independent of the commit hash because generated files are
# part of the commit whose tree they describe.
git diff --quiet
git diff --cached --quiet
python3 scripts/generate-dashboard.py --check
test -s qualification/capability-matrix.md
echo "capability matrix is current for the checked source tree"
