#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

remu=${REMU_BIN:-target/debug/remu}
artifact=qualification/starlark.json

"$remu" script \
    --file qualification/portfolio.star \
    --data riscv_cpu=qualification/riscv-cpu.json \
    --data arm_cpu=qualification/arm-cpu.json \
    --data xtensa_cpu=qualification/xtensa-cpu.json \
    --data reduction=qualification/reduction.json \
    --artifact "$artifact"

jq -e '
  .schema == "remu.starlark-assertion.v1" and
  .value == true and
  .result == "pass" and
  (.datasets | length == 4)
' "$artifact" >/dev/null

echo "bounded Starlark assertions passed; artifact: $artifact"
