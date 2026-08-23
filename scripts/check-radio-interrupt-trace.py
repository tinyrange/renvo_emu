#!/usr/bin/env python3
"""Validate one deterministic radio interrupt stream against the checked contract."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--chip", required=True)
    parser.add_argument("--workflow", required=True)
    parser.add_argument("--trace", type=Path, required=True)
    args = parser.parse_args()

    contract = json.loads(args.contract.read_text())
    expected = contract["workflows"][args.workflow]["expected"][args.chip]
    trace = json.loads(args.trace.read_text())
    if not isinstance(trace, list) or not trace:
        raise SystemExit(f"{args.trace}: interrupt trace is empty")

    expected_by_source = {entry["source"]: entry for entry in expected}
    observed_sources = {record.get("line") for record in trace}
    if observed_sources != set(expected_by_source):
        raise SystemExit(
            f"{args.trace}: sources {sorted(observed_sources)} do not match "
            f"contract {sorted(expected_by_source)}"
        )

    prior: dict[int, bool] = {}
    counts = {source: [0, 0] for source in expected_by_source}
    for index, record in enumerate(trace):
        source = record.get("line")
        expected_entry = expected_by_source.get(source)
        if expected_entry is None:
            raise SystemExit(f"{args.trace}: unexpected source {source} at record {index}")
        if record.get("source") != expected_entry["signal"]:
            raise SystemExit(
                f"{args.trace}: source {source} has signal {record.get('source')!r}, "
                f"expected {expected_entry['signal']!r}"
            )
        asserted = record.get("asserted")
        if not isinstance(asserted, bool):
            raise SystemExit(f"{args.trace}: record {index} has no Boolean level")
        if source in prior and prior[source] == asserted:
            raise SystemExit(
                f"{args.trace}: source {source} repeats level {asserted} at record {index}"
            )
        prior[source] = asserted
        counts[source][0 if asserted else 1] += 1

    for source, expected_entry in expected_by_source.items():
        assertions, deassertions = counts[source]
        minimum = expected_entry["minimum_pairs"]
        if assertions < minimum or assertions != deassertions or prior[source]:
            raise SystemExit(
                f"{args.trace}: source {source} has {assertions} assertions and "
                f"{deassertions} deassertions; expected at least {minimum} balanced pairs"
            )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
