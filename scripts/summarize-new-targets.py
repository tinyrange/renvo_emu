#!/usr/bin/env python3
"""Validate and summarize the software-only new-target qualification."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


CASES = [
    (
        "stm32f411re",
        list(b"STM32F411\n"),
        {
            "RCC": (0x40023800, 0x40023C00),
            "GPIOA": (0x40020000, 0x40020400),
            "USART2": (0x40004400, 0x40004800),
            "TIM2": (0x40000000, 0x40000400),
        },
    ),
    (
        "nrf52840",
        list(b"NRF52840\n"),
        {
            "GPIO P0": (0x50000500, 0x50000780),
            "UART0": (0x40002000, 0x40003000),
            "TIMER0": (0x40008000, 0x40009000),
        },
    ),
    (
        "esp32p4",
        list(b"ESP32P4\n"),
        {
            "GPIO": (0x500E0000, 0x500E1000),
            "UART0": (0x500CA000, 0x500CB000),
            "TIMG0": (0x500C2000, 0x500C3000),
        },
    ),
]


def load(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    digest.update(path.read_bytes())
    return digest.hexdigest()


def immutable_image_id(value: object) -> bool:
    return (
        isinstance(value, str)
        and value.startswith("sha256:")
        and len(value) == 71
        and all(character in "0123456789abcdef" for character in value[7:])
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source", action="append", default=[], type=Path)
    args = parser.parse_args()

    summaries = []
    for target, expected_uart, required_ranges in CASES:
        build_path = args.root / f"{target}-build.json"
        build = load(build_path)
        if not isinstance(build, dict) or build.get("schema") != "remu.build-artifact.v1":
            raise ValueError(f"{build_path}: invalid build artifact")
        if build.get("exit_code") != 0 or build.get("timed_out"):
            raise ValueError(f"{build_path}: compilation failed")
        if not immutable_image_id(build.get("image_id")):
            raise ValueError(f"{build_path}: compiler did not resolve to an immutable image ID")

        run_a = args.root / f"{target}-run-a"
        run_b = args.root / f"{target}-run-b"
        result_a_path = run_a / "result.json"
        result_b_path = run_b / "result.json"
        result_a = load(result_a_path)
        result_b = load(result_b_path)
        if result_a != result_b:
            raise ValueError(f"{target}: repeated results diverged")
        if not isinstance(result_a, dict) or any(
            (
                result_a.get("target") != target,
                result_a.get("reason") != "Halted",
                result_a.get("exit_code") != 0,
                result_a.get("uart") != expected_uart,
            )
        ):
            raise ValueError(f"{target}: functional oracle failed")

        vcd_a = run_a / "signals.vcd"
        vcd_b = run_b / "signals.vcd"
        bus_a = run_a / "bus.json"
        bus_b = run_b / "bus.json"
        if vcd_a.read_bytes() != vcd_b.read_bytes():
            raise ValueError(f"{target}: repeated VCD traces diverged")
        if bus_a.read_bytes() != bus_b.read_bytes():
            raise ValueError(f"{target}: repeated bus traces diverged")
        bus = load(bus_a)
        if not isinstance(bus, list):
            raise ValueError(f"{target}: bus trace is not an event list")
        addresses = {
            event.get("address")
            for event in bus
            if isinstance(event, dict) and event.get("kind") in {"Read", "Write"}
        }
        observed = []
        for name, (start, end) in required_ranges.items():
            if not any(isinstance(address, int) and start <= address < end for address in addresses):
                raise ValueError(f"{target}: no native access observed for {name}")
            observed.append(name)

        summaries.append(
            {
                "id": target,
                "toolchain": build.get("toolchain"),
                "toolchain_reference": build.get("image"),
                "toolchain_image_id": build.get("image_id"),
                "build_artifact": str(build_path),
                "build_sha256": sha256(build_path),
                "result_artifact": str(result_a_path),
                "result_sha256": sha256(result_a_path),
                "vcd_sha256": sha256(vcd_a),
                "bus_sha256": sha256(bus_a),
                "trace_digest": result_a.get("trace_digest"),
                "uart": expected_uart,
                "observed_native_blocks": observed,
                "deterministic_replay": True,
                "status": "pass",
            }
        )

    source_digest = hashlib.sha256()
    for path in sorted(args.source):
        source_digest.update(str(path).encode("utf-8"))
        source_digest.update(b"\0")
        source_digest.update(path.read_bytes())
        source_digest.update(b"\0")
    output = {
        "schema": "remu.new-target-qualification-result.v1",
        "result": "pass",
        "software_only": True,
        "hardware_access": False,
        "rf_transmission": False,
        "network_disabled_for_compilation": True,
        "source_sha256": source_digest.hexdigest(),
        "target_count": len(summaries),
        "targets": summaries,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
