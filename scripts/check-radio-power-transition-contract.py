#!/usr/bin/env python3
"""Validate the software-only ESP radio power-transition stress contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONTRACT = ROOT / "qualification/radio/power-transition-contract.json"
EXPECTED_PROTOCOLS = {
    "esp32c6": {"wifi", "bluetooth-le", "ieee802154"},
    "esp32s3": {"wifi", "bluetooth-le"},
}


def fail(message: str) -> None:
    raise ValueError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def load_json(path: Path) -> dict[str, object]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot load {path}: {error}")
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    return value


def validate_contract(contract: dict[str, object]) -> None:
    require(
        contract.get("schema") == "remu.radio-power-transition-contract.v1",
        "unexpected power-transition contract schema",
    )
    require(contract.get("issue") == 352, "power-transition contract must own issue 352")
    boundary = contract.get("execution_boundary")
    require(isinstance(boundary, dict), "execution_boundary must be an object")
    require(boundary.get("mode") == "software-emulator-only", "contract must be emulator-only")
    for forbidden in ("physical_rf_allowed", "hardware_test_allowed", "host_networking_allowed"):
        require(boundary.get(forbidden) is False, f"{forbidden} must remain false")
    require(boundary.get("symbol_dispatch_allowed") is False, "symbol dispatch must remain disabled")
    require(
        boundary.get("deterministic_replay_required") is True,
        "deterministic replay must remain mandatory",
    )

    bounded = contract.get("bounded_stress")
    require(isinstance(bounded, dict), "bounded_stress must be an object")
    require(bounded.get("low_power_cycles_per_chip") == 256, "low-power torture must retain 256 cycles")
    require(
        bounded.get("active_airtime_transitions_per_chip") == 128,
        "active-airtime torture must retain 128 transitions",
    )
    require(bounded.get("openthread_sleep_wake_cycles") == 16, "OpenThread must retain 16 cycles")
    require(bounded.get("vendor_workflow_count") == 7, "all seven bounded vendor workflows are required")
    require(
        bounded.get("requirements_own_instruction_limits") is True,
        "each pinned workflow must retain its own instruction limit",
    )

    chips = contract.get("chips")
    require(isinstance(chips, dict), "chips must be an object")
    require(set(chips) == set(EXPECTED_PROTOCOLS), "contract must cover exactly C6 and S3")
    for chip, expected_protocols in EXPECTED_PROTOCOLS.items():
        chip_contract = chips.get(chip)
        require(isinstance(chip_contract, dict), f"{chip} contract must be an object")
        protocols = chip_contract.get("protocols")
        require(isinstance(protocols, dict), f"{chip} protocols must be an object")
        require(set(protocols) == expected_protocols, f"{chip} protocol coverage is incomplete")
        require(
            chip_contract.get("coexistence_workflow")
            == f"scripts/qualify-esp-radio-coexistence-vendor.sh {chip}",
            f"{chip} must retain genuine coexistence qualification",
        )
        for protocol, evidence in protocols.items():
            require(isinstance(evidence, dict), f"{chip} {protocol} evidence must be an object")
            require(
                str(evidence.get("genuine_firmware_workflow", "")).startswith("scripts/qualify-esp-radio-"),
                f"{chip} {protocol} must name a genuine-firmware workflow",
            )
            require(evidence.get("active_traffic_required") is True, f"{chip} {protocol} needs active traffic")
            require(
                evidence.get("calibration_restoration_required") is True,
                f"{chip} {protocol} needs calibration restoration",
            )
            require(bool(evidence.get("interrupt_sources")), f"{chip} {protocol} needs interrupt sources")
            require(bool(evidence.get("fault_semantics")), f"{chip} {protocol} needs explicit fault semantics")

    focused = contract.get("focused_tests")
    require(isinstance(focused, list) and len(focused) >= 6, "focused stress/negative tests are incomplete")
    require(len(focused) == len(set(focused)), "focused test names must be unique")


def get_bool(document: dict[str, object], *path: str) -> bool:
    value: object = document
    for key in path:
        require(isinstance(value, dict) and key in value, f"missing artifact field {'.'.join(path)}")
        value = value[key]
    return value is True


def validate_artifacts(artifact_root: Path) -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for chip in EXPECTED_PROTOCOLS:
        rom_path = artifact_root / "rom" / chip / "summary.json"
        rom = load_json(rom_path)
        require(rom.get("chip") == chip, f"{rom_path} has wrong chip")
        require(get_bool(rom, "requirement", "requires_firmware_observed_calibration"), f"{chip} Wi-Fi calibration missing")
        require(get_bool(rom, "observed", "deterministic_replay"), f"{chip} Wi-Fi replay missing")
        if chip == "esp32c6":
            require(get_bool(rom, "observed", "native_c6_itwt_observed"), "C6 TWT wake missing")
        records.append(record("wifi", chip, rom_path, rom))

        ble_path = artifact_root / "ble" / chip / "summary.json"
        ble = load_json(ble_path)
        require(ble.get("chip") == chip, f"{ble_path} has wrong chip")
        require(get_bool(ble, "observed", "ble_modem_sleep_qualified"), f"{chip} BLE sleep missing")
        require(get_bool(ble, "observed", "deterministic_replay"), f"{chip} BLE replay missing")
        minimum_tx = ble.get("requirement", {}).get("minimum_sleep_native_tx") if isinstance(ble.get("requirement"), dict) else None
        observed_tx = ble.get("observed", {}).get("sleep_native_tx") if isinstance(ble.get("observed"), dict) else None
        require(isinstance(minimum_tx, int) and isinstance(observed_tx, int) and observed_tx >= minimum_tx, f"{chip} BLE sleep traffic is below its bound")
        records.append(record("bluetooth-le", chip, ble_path, ble))

        coex_path = artifact_root / "coexistence" / chip / "qualification.json"
        coex = load_json(coex_path)
        require(coex.get("chip") == chip, f"{coex_path} has wrong chip")
        require(get_bool(coex, "evidence", "wifi_ble_coexistence_owned"), f"{chip} coexistence ownership missing")
        resets = coex.get("evidence", {}).get("firmware_reset_boundaries") if isinstance(coex.get("evidence"), dict) else None
        require(isinstance(resets, int) and resets >= 4, f"{chip} coexistence reset evidence is incomplete")
        records.append(record("coexistence", chip, coex_path, coex))

    thread_path = artifact_root / "openthread" / "esp32c6" / "summary.json"
    thread = load_json(thread_path)
    require(thread.get("chip") == "esp32c6", f"{thread_path} has wrong chip")
    cycles = thread.get("observed", {}).get("sleep_wake_cycles_completed") if isinstance(thread.get("observed"), dict) else None
    require(cycles == 16, "C6 OpenThread did not complete all 16 sleep/wake cycles")
    require(get_bool(thread, "observed", "deterministic_result_replay"), "OpenThread result replay missing")
    require(get_bool(thread, "observed", "deterministic_rf_replay"), "OpenThread RF replay missing")
    records.append(record("ieee802154", "esp32c6", thread_path, thread))
    return records


def record(protocol: str, chip: str, path: Path, document: dict[str, object]) -> dict[str, object]:
    return {
        "chip": chip,
        "protocol": protocol,
        "schema": document.get("schema"),
        "artifact": str(path),
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        contract = load_json(args.contract)
        validate_contract(contract)
        records = validate_artifacts(args.artifacts) if args.artifacts else []
        if args.output:
            require(args.artifacts is not None, "--output requires --artifacts")
            summary = {
                "schema": "remu.radio-power-transition-qualification.v1",
                "contract_sha256": hashlib.sha256(args.contract.read_bytes()).hexdigest(),
                "software_emulator_only": True,
                "deterministic_replay": True,
                "evidence": records,
            }
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    except ValueError as error:
        print(f"radio power-transition contract error: {error}", file=sys.stderr)
        return 1
    print("radio power-transition contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
