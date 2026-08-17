#!/usr/bin/env python3
"""Fail closed if the checked-in C6 RF register/error references drift."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT / "qualification/radio/c6-rf-register-reference.json"
CORPUS = ROOT / "qualification/radio/c6-rf-error-corpus.json"
EXPECTED_ERRORS = {
    "domain-ready",
    "rf-pll-lock",
    "rf-calibration",
    "rf-channel",
    "rf-power",
    "rf-bandwidth",
    "rf-frontend",
}
EXPECTED_TRACE_REGIONS = {
    "esp32c6.power-detector",
    "esp32c6.phy-mac-registers",
    "esp32c6.wifi-mac-registers",
    "esp32c6.phy-baseband-registers",
    "esp32c6.phy-front-end-registers",
    "esp32c6.modem-syscon",
    "esp32c6.phy-registers",
    "esp32c6.modem-lpcon",
    "esp32c6.i2c-ana-mst",
    "esp32c6.phy-i2c-command-memory",
}
EXPECTED_IMPLEMENTED_REGIONS = {
    "esp32c6.power-detector",
    "esp32c6.ble-baseband-registers",
    "esp32c6.ieee802154",
    "esp32c6.wifi-mac-registers",
    "esp32c6.ble-control-registers",
    "esp32c6.modem-syscon",
    "esp32c6.phy-registers",
    "esp32c6.ble-modem-registers",
    "esp32c6.modem-lpcon",
    "esp32c6.i2c-ana-mst",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"C6 RF reference check failed: {message}")


def main() -> None:
    reference = json.loads(REFERENCE.read_text())
    corpus = json.loads(CORPUS.read_text())
    require(reference["schema"] == "remu.c6-rf-register-reference.v1", "reference schema")
    require(corpus["schema"] == "remu.c6-rf-error-corpus.v1", "corpus schema")
    require(reference["scope"] == corpus["scope"] == "software-emulator-only", "scope")

    registers = reference["registers"]
    addresses = [int(item["address"], 16) for item in registers]
    require(addresses == sorted(addresses), "registers are not address-sorted")
    require(len(addresses) == len(set(addresses)), "duplicate register addresses")
    require(all(item["trace_observed"] or item["semantically_implemented"] for item in registers), "entry has no inclusion reason")
    require(all(item["name"] and item["technical_function"] != "unknown; factual observed values only" for item in registers if item["semantically_implemented"]), "implemented register lacks documented semantics")
    require(all(item["implementation_source"] for item in registers if item["semantically_implemented"]), "implemented register lacks source provenance")
    require(all(item["implementation_source"] is None for item in registers if not item["semantically_implemented"]), "trace-only register claims implementation source")
    require(all(item["trace_observed"] for item in registers if not item["semantically_implemented"]), "unknown register was not trace-observed")
    require(all(item["read_count"] + item["write_count"] > 0 for item in registers if item["trace_observed"]), "observed register has no accesses")
    require(all(item["access_observed"] == "not-observed" for item in registers if not item["trace_observed"]), "implemented-only register claims trace access")

    stats = reference["statistics"]
    require(stats["registers"] == len(registers), "total register count")
    require(stats["records"] == 121321, "pinned non-UART trace record count")
    require(stats["trace_observed_registers"] == 478, "pinned trace register count")
    require(stats["distinct_observed_values"] == 6872, "pinned observed-value count")
    require(stats["trace_observed_registers"] == sum(item["trace_observed"] for item in registers), "observed statistic")
    require(stats["semantically_implemented_registers"] == sum(item["semantically_implemented"] for item in registers), "implemented statistic")
    require(stats["implemented_only_registers"] == sum(item["semantically_implemented"] and not item["trace_observed"] for item in registers), "implemented-only statistic")
    require(stats["explicit_unknowns"] == sum(item["name"] is None for item in registers), "unknown statistic")

    trace_regions = {item["region"] for item in registers if item["trace_observed"]}
    implemented_regions = {item["region"] for item in registers if item["semantically_implemented"]}
    require(trace_regions == EXPECTED_TRACE_REGIONS, "trace region coverage")
    require(implemented_regions == EXPECTED_IMPLEMENTED_REGIONS, "implemented region coverage")

    cases = corpus["cases"]
    require([case["mutation_id"] for case in cases] == list(range(8)), "mutation IDs")
    require(cases[0]["expected"] == "accepted", "valid reference case")
    require({case["expected"] for case in cases[1:]} == EXPECTED_ERRORS, "causal error coverage")
    require(set(corpus["legality_order"]) == EXPECTED_ERRORS, "legality ordering coverage")
    require(corpus["generator"]["seed_hex"] == "0x356c6f7261636c65", "fuzz seed")
    require(corpus["generator"]["iterations"] == 2048, "fuzz iteration count")

    print(
        "C6 RF register reference: "
        f"{stats['registers']} union / {stats['trace_observed_registers']} observed / "
        f"{stats['semantically_implemented_registers']} implemented; "
        f"{len(EXPECTED_ERRORS)} causal error paths"
    )


if __name__ == "__main__":
    main()
