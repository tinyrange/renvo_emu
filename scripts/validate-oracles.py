#!/usr/bin/env python3
"""Validate checked Renvo Emulator hardware/differential oracle fixtures."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
FIXTURE_ROOT = ROOT / "qualification" / "oracles" / "fixtures"
SCHEMA = "remu.oracle.v1"
HEX32 = re.compile(r"^0x[0-9a-f]{8}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


class ValidationError(Exception):
    """A fixture failed a replayable validation check."""


def fail(message: str) -> None:
    raise ValidationError(message)


def require(value: Any, kind: type, path: str) -> Any:
    if not isinstance(value, kind):
        fail(f"{path}: expected {kind.__name__}")
    return value


def nonempty(value: Any, path: str) -> str:
    require(value, str, path)
    if not value:
        fail(f"{path}: must not be empty")
    return value


def repository_path(value: Any, path: str) -> Path:
    """Resolve a repository-relative path without allowing traversal."""
    relative = Path(nonempty(value, path))
    if relative.is_absolute() or ".." in relative.parts:
        fail(f"{path}: unsafe path {relative}")
    return ROOT / relative


def sha256_text(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_paths(entries: list[str]) -> list[tuple[str, Path]]:
    resolved: list[tuple[str, Path]] = []
    for entry in entries:
        relative = Path(entry)
        if relative.is_absolute() or ".." in relative.parts:
            fail(f"firmware.source_files: unsafe path {entry!r}")
        path = ROOT / relative
        if not path.exists():
            fail(f"firmware.source_files: missing {entry}")
        if path.is_file():
            resolved.append((entry, path))
            continue
        files = sorted(child for child in path.rglob("*") if child.is_file())
        if not files:
            fail(f"firmware.source_files: empty directory {entry}")
        resolved.extend(
            (str(child.relative_to(ROOT)), child) for child in files
        )
    unique = dict(resolved)
    return sorted(unique.items())


def source_digest(entries: list[str]) -> str:
    digest = hashlib.sha256()
    for relative, path in source_paths(entries):
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def manifest_values(path: Path) -> dict[str, str]:
    if not path.is_file():
        fail(f"comparison.reference: missing {path.relative_to(ROOT)}")
    values: dict[str, str] = {}
    with path.open("r", encoding="utf-8", newline="") as stream:
        for row in csv.DictReader(stream, delimiter="\t"):
            case_id = row.get("case_id")
            expected = row.get("expected_hex")
            if not case_id or not expected:
                fail(f"comparison.reference: malformed row {row!r}")
            values[case_id] = expected.lower()
    if not values:
        fail(f"comparison.reference: no cases in {path}")
    return values


def canonical_observation_digest(records: list[dict[str, str]]) -> str:
    payload = "".join(
        f"{record['case_id']} {record['value_hex'].lower()}\n"
        for record in records
    )
    return hashlib.sha256(payload.encode("ascii")).hexdigest()


def validate_fixture(path: Path) -> None:
    try:
        fixture = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"{path}: cannot read JSON: {error}")
    require(fixture, dict, "fixture")

    if fixture.get("schema") != SCHEMA:
        fail(f"{path}: schema must be {SCHEMA}")
    fixture_id = nonempty(fixture.get("fixture_id"), "fixture_id")
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]+", fixture_id):
        fail(f"fixture_id: invalid {fixture_id!r}")
    if path.stem != fixture_id:
        fail(f"fixture_id: {fixture_id!r} does not match {path.name!r}")
    if fixture.get("status") not in {
        "hardware_capture",
        "external_emulator",
        "remu_reference",
    }:
        fail(f"{fixture_id}: unsupported status")

    target = require(fixture.get("target"), dict, f"{fixture_id}.target")
    for key in ("architecture", "machine", "board"):
        nonempty(target.get(key), f"{fixture_id}.target.{key}")

    hardware = require(fixture.get("hardware"), dict, f"{fixture_id}.hardware")
    for key in ("board", "mcu", "revision"):
        nonempty(hardware.get(key), f"{fixture_id}.hardware.{key}")
    for key in ("probe", "toolchain"):
        require(hardware.get(key), dict, f"{fixture_id}.hardware.{key}")

    firmware = require(fixture.get("firmware"), dict, f"{fixture_id}.firmware")
    entries = require(
        firmware.get("source_files"), list, f"{fixture_id}.firmware.source_files"
    )
    if not entries or not all(isinstance(entry, str) for entry in entries):
        fail(f"{fixture_id}.firmware.source_files: expected non-empty strings")
    expected_source = nonempty(
        firmware.get("source_sha256"), f"{fixture_id}.firmware.source_sha256"
    )
    if not SHA256.fullmatch(expected_source):
        fail(f"{fixture_id}.firmware.source_sha256: invalid SHA-256")
    actual_source = source_digest(entries)
    if actual_source != expected_source:
        fail(
            f"{fixture_id}: source digest mismatch; expected {expected_source}, "
            f"got {actual_source}"
        )
    image_hash = nonempty(
        firmware.get("image_sha256"), f"{fixture_id}.firmware.image_sha256"
    )
    if not SHA256.fullmatch(image_hash):
        fail(f"{fixture_id}.firmware.image_sha256: invalid SHA-256")
    require(firmware.get("build"), dict, f"{fixture_id}.firmware.build")

    initial = require(
        fixture.get("initial_state"), dict, f"{fixture_id}.initial_state"
    )
    for key in ("reset", "registers", "memory"):
        require(initial.get(key), dict, f"{fixture_id}.initial_state.{key}")

    stimulus = require(fixture.get("stimulus"), dict, f"{fixture_id}.stimulus")
    nonempty(stimulus.get("kind"), f"{fixture_id}.stimulus.kind")
    steps = require(stimulus.get("steps"), list, f"{fixture_id}.stimulus.steps")
    if not steps or not all(isinstance(step, dict) for step in steps):
        fail(f"{fixture_id}.stimulus.steps: expected non-empty objects")

    observations = require(
        fixture.get("observations"), dict, f"{fixture_id}.observations"
    )
    nonempty(observations.get("format"), f"{fixture_id}.observations.format")
    records = require(
        observations.get("records"), list, f"{fixture_id}.observations.records"
    )
    if not records or not all(isinstance(record, dict) for record in records):
        fail(f"{fixture_id}.observations.records: expected non-empty objects")
    seen: set[str] = set()
    normalized: list[dict[str, str]] = []
    for index, record in enumerate(records):
        prefix = f"{fixture_id}.observations.records[{index}]"
        case_id = nonempty(record.get("case_id"), f"{prefix}.case_id")
        if case_id in seen:
            fail(f"{prefix}.case_id: duplicate {case_id}")
        seen.add(case_id)
        value = nonempty(record.get("value_hex"), f"{prefix}.value_hex").lower()
        if not HEX32.fullmatch(value):
            fail(f"{prefix}.value_hex: expected 32-bit hex value")
        reference = nonempty(
            record.get("reference_value_hex"), f"{prefix}.reference_value_hex"
        ).lower()
        if not HEX32.fullmatch(reference):
            fail(f"{prefix}.reference_value_hex: expected 32-bit hex value")
        normalized.append({"case_id": case_id, "value_hex": value})

    comparison = require(
        fixture.get("comparison"), dict, f"{fixture_id}.comparison"
    )
    reference_path = repository_path(
        comparison.get("reference"), f"{fixture_id}.comparison.reference"
    )
    expected_values = manifest_values(reference_path)
    expected_count = comparison.get("expected_observations")
    if not isinstance(expected_count, int) or isinstance(expected_count, bool):
        fail(f"{fixture_id}.comparison.expected_observations: expected integer")
    if expected_count < 1 or len(records) != expected_count:
        fail(
            f"{fixture_id}: expected {expected_count} observations, got {len(records)}"
        )
    for index, record in enumerate(records):
        expected = expected_values.get(record["case_id"])
        if expected is None:
            fail(f"{fixture_id}.observations.records[{index}]: unknown case")
        if record["reference_value_hex"].lower() != expected:
            fail(
                f"{fixture_id}: reference mismatch for {record['case_id']}; "
                f"expected manifest {expected}, got {record['reference_value_hex']}"
            )
        if record["value_hex"].lower() != expected:
            fail(
                f"{fixture_id}: observation mismatch for {record['case_id']}; "
                f"expected {expected}, got {record['value_hex']}"
            )
    nonempty(comparison.get("equivalence"), f"{fixture_id}.comparison.equivalence")
    require(comparison.get("timing"), dict, f"{fixture_id}.comparison.timing")

    canonical = nonempty(
        observations.get("canonical_sha256"),
        f"{fixture_id}.observations.canonical_sha256",
    )
    if not SHA256.fullmatch(canonical):
        fail(f"{fixture_id}.observations.canonical_sha256: invalid SHA-256")
    actual_canonical = canonical_observation_digest(normalized)
    if actual_canonical != canonical:
        fail(
            f"{fixture_id}: canonical capture digest mismatch; expected {canonical}, "
            f"got {actual_canonical}"
        )

    provenance = require(
        fixture.get("provenance"), dict, f"{fixture_id}.provenance"
    )
    for key in ("captured_at", "capture_method"):
        nonempty(provenance.get(key), f"{fixture_id}.provenance.{key}")
    capture_hash = nonempty(
        provenance.get("capture_source_sha256"),
        f"{fixture_id}.provenance.capture_source_sha256",
    )
    if not SHA256.fullmatch(capture_hash):
        fail(f"{fixture_id}.provenance.capture_source_sha256: invalid SHA-256")

    print(
        f"oracle fixture passed: {fixture_id} "
        f"({len(records)} observations, {fixture['status']})"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--fixture",
        action="append",
        type=Path,
        help="fixture JSON path (default: all qualification/oracles/fixtures/*.json)",
    )
    arguments = parser.parse_args()
    fixtures = arguments.fixture or sorted(FIXTURE_ROOT.glob("*.json"))
    if not fixtures:
        print("no oracle fixtures found", file=sys.stderr)
        return 1
    try:
        for fixture in fixtures:
            validate_fixture(
                fixture if fixture.is_absolute() else ROOT / fixture
            )
    except ValidationError as error:
        print(f"oracle validation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
