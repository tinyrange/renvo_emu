#!/usr/bin/env python3
"""Validate benchmark artifacts and compare them with an optional baseline."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent


class BudgetError(Exception):
    pass


def load(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BudgetError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise BudgetError(f"{path}: expected an object")
    return value


def percent_limit(value: float, limit: float) -> float:
    return value * (1.0 + limit / 100.0)


def require_run_fields(run: dict[str, Any], label: str) -> None:
    for field in (
        "machine_instructions",
        "abstract_ticks",
        "host_elapsed_seconds",
        "host_iterations_per_second",
        "peak_rss_bytes",
        "result_artifact_bytes",
    ):
        if field not in run:
            raise BudgetError(f"{label}: missing {field}")
    if run["machine_instructions"] <= 0 or run["abstract_ticks"] <= 0:
        raise BudgetError(f"{label}: execution counters must be positive")
    if run["host_elapsed_seconds"] <= 0 or run["host_iterations_per_second"] <= 0:
        raise BudgetError(f"{label}: host throughput counters must be positive")
    if run["peak_rss_bytes"] < 0 or run["result_artifact_bytes"] < 0:
        raise BudgetError(f"{label}: resource counters must not be negative")


def check_coremark(current: dict[str, Any], baseline: dict[str, Any] | None, budgets: dict[str, Any]) -> None:
    if current.get("schema") not in {
        "renvo.coremark-qualification.v1",
        "remu.coremark-qualification.v1",
    }:
        raise BudgetError("current artifact is not a supported CoreMark qualification schema")
    runs = current.get("runs")
    if not isinstance(runs, list) or not runs:
        raise BudgetError("current artifact contains no runs")
    if current.get("host", {}).get("model") in (None, ""):
        raise BudgetError("current artifact does not record host identity")
    for run in runs:
        require_run_fields(run, f"{run.get('id')}/{run.get('variant')}")
        expected_action_score = round(
            run["iterations"] * 1_000_000 / run["abstract_ticks"], 6
        )
        if abs(run.get("iterations_per_million_actions", -1) - expected_action_score) > 0.000001:
            raise BudgetError(f"{run.get('id')}: action-normalized score is inconsistent")

    modes = current.get("observability_modes", [])
    if not isinstance(modes, list):
        raise BudgetError("observability_modes must be an array")
    for mode in modes:
        if not isinstance(mode, dict) or mode.get("mode") not in {
            "no-trace",
            "vcd",
            "coverage",
            "bus-log",
        }:
            raise BudgetError("observability_modes contains an unknown mode")
        measurement = mode.get("measurement", {})
        if measurement.get("status") != "pass":
            raise BudgetError(f"observability mode failed: {mode.get('mode')}")
        if measurement.get("peak_rss_bytes", 0) <= 0:
            raise BudgetError(f"observability mode has no RSS measurement: {mode.get('mode')}")
        if mode.get("mode") == "bus-log" and not mode.get("streaming"):
            raise BudgetError("bus-log observability mode is not marked streaming")

    if baseline is None:
        return
    if baseline.get("schema") != current.get("schema"):
        raise BudgetError("baseline schema differs from current artifact")
    baseline_runs = {
        (run.get("id"), run.get("variant")): run for run in baseline.get("runs", [])
    }
    comparison = budgets["comparison"]
    for run in runs:
        key = (run.get("id"), run.get("variant"))
        previous = baseline_runs.get(key)
        if previous is None:
            continue
        label = f"{key[0]}/{key[1]}"
        if "host_elapsed_seconds" in previous and run["host_elapsed_seconds"] > percent_limit(
            previous["host_elapsed_seconds"], comparison["wall_time_regression_percent"]
        ):
            raise BudgetError(f"{label}: wall time exceeds noise budget")
        if "peak_rss_bytes" in previous and run["peak_rss_bytes"] > percent_limit(
            previous["peak_rss_bytes"], comparison["peak_rss_regression_percent"]
        ):
            raise BudgetError(f"{label}: peak RSS exceeds noise budget")
        if "result_artifact_bytes" in previous and run["result_artifact_bytes"] > percent_limit(
            previous["result_artifact_bytes"], comparison["artifact_growth_percent"]
        ):
            raise BudgetError(f"{label}: result artifact exceeds growth budget")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("current", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument(
        "--budgets",
        type=Path,
        default=ROOT / "qualification/benchmarks/budgets.json",
    )
    arguments = parser.parse_args()
    try:
        budgets = load(arguments.budgets)
        current = load(arguments.current)
        baseline = load(arguments.baseline) if arguments.baseline else None
        check_coremark(current, baseline, budgets)
    except BudgetError as error:
        print(f"benchmark budget check failed: {error}", file=sys.stderr)
        return 1
    print(f"benchmark budget check passed: {arguments.current}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
