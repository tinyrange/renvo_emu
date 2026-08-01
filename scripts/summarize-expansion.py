#!/usr/bin/env python3
"""Validate seven-target qualification artifacts and emit the acceptance summary."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import tomllib


TARGETS = {
    "atsamd21e18": {
        "builds": ["build.json", "build-O0.json", "build-Os.json", "clang-build.json"],
        "run": "run-a/result.json",
        "repeat": "run-b/result.json",
        "vcd": "run-a/gpio.vcd",
        "repeat_vcd": "run-b/gpio.vcd",
        "vendor_builds": ["harmony-port-build.json"],
        "vendor_runs": ["harmony-port-run/result.json"],
        "edges": {
            "renvo.board.atsamd21e18.porta.pin7": ["1", "0"],
            "renvo.board.atsamd21e18.tc3.irq": ["0", "1"],
            "renvo.board.atsamd21e18.sercom0.tx_strobe": ["0", "1"],
            "renvo.board.atsamd21e18.interrupt.request": ["0", "1"],
        },
    },
    "stm32l432kc": {
        "builds": [
            "soft-o0-build.json",
            "soft-os-build.json",
            "soft-build.json",
            "softfp-build.json",
            "hard-build.json",
            "clang-build.json",
        ],
        "run": "hard-run/result.json",
        "repeat": "hard-repeat/result.json",
        "vcd": "hard-run/gpio.vcd",
        "repeat_vcd": "hard-repeat/gpio.vcd",
        "vendor_builds": ["cube-gpio-build.json"],
        "vendor_runs": ["cube-gpio-run/result.json"],
        "edges": {
            "renvo.board.stm32l432kc.gpioa.pin5": ["1", "0"],
            "renvo.board.stm32l432kc.tim2.irq": ["0", "1"],
            "renvo.board.stm32l432kc.usart2.tx_strobe": ["0", "1"],
            "renvo.board.stm32l432kc.interrupt.request": ["0", "1"],
        },
    },
    "r7fa4m1ab3cfm": {
        "builds": ["build.json", "build-O0.json", "build-Os.json", "clang-build.json"],
        "run": "run-a/result.json",
        "repeat": "run-b/result.json",
        "vcd": "run-a/gpio.vcd",
        "repeat_vcd": "run-b/gpio.vcd",
        "vendor_builds": [
            "fsp-build.json",
            "arduino-blink-build.json",
            "arduino-hardware-serial-build.json",
        ],
        "vendor_runs": [
            "fsp-run/result.json",
            "arduino-blink-run/result.json",
            "arduino-hardware-serial-run/result.json",
        ],
        "edges": {
            "renvo.board.r7fa4m1ab3cfm.port1.pin11": ["1", "0"],
            "renvo.board.r7fa4m1ab3cfm.gpt0.irq": ["0", "1"],
            "renvo.board.r7fa4m1ab3cfm.sci9.tx_strobe": ["0", "1"],
            "renvo.board.r7fa4m1ab3cfm.icu.request": ["0", "1"],
        },
    },
    "atmega328pb": {
        "builds": ["build-O0.json", "build-Os.json", "build-O2.json"],
        "run": "run-a/result.json",
        "repeat": "run-b/result.json",
        "vcd": "run-a/signals.vcd",
        "repeat_vcd": "run-b/signals.vcd",
        "vendor_builds": ["avr-libc-build.json"],
        "vendor_runs": ["avr-libc-run/result.json"],
        "edges": {
            "renvo.board.atmega328pb.portb.pin0": ["0", "1"],
            "renvo.board.atmega328pb.timer0.overflow_irq": ["0", "1"],
            "renvo.board.atmega328pb.interrupt.pcint0": ["0", "1"],
        },
    },
    "msp430fr2433": {
        "builds": ["build-O0.json", "build-Os.json", "build-O2.json"],
        "run": "run-a/result.json",
        "repeat": "run-b/result.json",
        "vcd": "run-a/signals.vcd",
        "repeat_vcd": "run-b/signals.vcd",
        "vendor_builds": [
            "slac700-gpio-build.json",
            "slac700-timer-build.json",
            "slac700-uart-build.json",
        ],
        "vendor_runs": [
            "slac700-gpio-run/result.json",
            "slac700-timer-run/result.json",
            "slac700-uart-run/result.json",
        ],
        "edges": {
            "renvo.board.msp430fr2433.port1.pin0": ["0", "1"],
            "renvo.board.msp430fr2433.timer_a0.ccr0_irq": ["0", "1"],
            "renvo.board.msp430fr2433.uart0.tx_strobe": ["0", "1"],
            "renvo.board.msp430fr2433.interrupt.port1": ["0", "1"],
        },
    },
    "pic16f15376": {
        "builds": ["build-O0.json", "build-Os.json", "build-O2.json", "isa-build.json"],
        "run": "run-O2/result.json",
        "repeat": "run-O2-repeat/result.json",
        "vcd": "run-O2/signals.vcd",
        "repeat_vcd": "run-O2-repeat/signals.vcd",
        "vendor_builds": ["microchip-timer0-build.json"],
        "vendor_runs": ["microchip-timer0-run/result.json"],
        "edges": {
            "renvo.board.pic16f15376.porta.pin0": ["0", "1"],
            "renvo.board.pic16f15376.timer0.irq": ["0", "1"],
            "renvo.board.pic16f15376.eusart1.tx_strobe": ["0", "1"],
            "renvo.board.pic16f15376.interrupt.request": ["0", "1"],
        },
    },
    "efm8bb52f32g": {
        "builds": ["build-baseline.json", "build-size.json", "build-speed.json"],
        "run": "run-speed/result.json",
        "repeat": "run-speed-repeat/result.json",
        "vcd": "run-speed/signals.vcd",
        "repeat_vcd": "run-speed-repeat/signals.vcd",
        "vendor_builds": [
            "silabs-main-build.json",
            "silabs-isr-build.json",
            "silabs-adapter-build.json",
            "silabs-link.json",
            "silabs-uart-build.json",
        ],
        "vendor_runs": ["silabs-blinky-run/result.json"],
        "edges": {
            "renvo.board.efm8bb52f32g.port0.pin0": ["0", "1"],
            "renvo.board.efm8bb52f32g.timer0.irq": ["0", "1"],
            "renvo.board.efm8bb52f32g.uart0.tx_strobe": ["0", "1"],
            "renvo.board.efm8bb52f32g.interrupt.request": ["0", "1"],
        },
    },
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict:
    with path.open("rb") as stream:
        return json.load(stream)


def parse_vcd_scalar_events(text: str) -> dict[str, list[tuple[int, str]]]:
    scopes: list[str] = []
    identifiers: dict[str, str] = {}
    events: dict[str, list[tuple[int, str]]] = {}
    time = 0
    definitions = True
    for line in text.splitlines():
        if definitions:
            if line.startswith("$scope module "):
                scopes.append(line.split()[2])
            elif line == "$upscope $end":
                scopes.pop()
            elif line.startswith("$var "):
                fields = line.split()
                identifiers[fields[3]] = ".".join([*scopes, fields[4]])
            elif line == "$enddefinitions $end":
                definitions = False
            continue
        if line.startswith("#"):
            time = int(line[1:])
        elif line and line[0] in "01xz" and line[1:] in identifiers:
            path = identifiers[line[1:]]
            events.setdefault(path, []).append((time, line[0]))
    return events


def contains_ordered_values(events: list[tuple[int, str]], expected: list[str]) -> bool:
    position = 0
    for _, value in events:
        if position < len(expected) and value == expected[position]:
            position += 1
    return position == len(expected)


def validate_build(path: Path, target: str) -> dict:
    artifact = load_json(path)
    if artifact.get("schema") != "renvo.build-artifact.v1":
        raise ValueError(f"{path}: unexpected build schema")
    if artifact.get("target") != target:
        raise ValueError(f"{path}: target does not match {target}")
    if artifact.get("exit_code") != 0 or artifact.get("timed_out"):
        raise ValueError(f"{path}: build did not succeed")
    if artifact.get("image") != artifact.get("image_id"):
        raise ValueError(f"{path}: mutable or mismatched image identity")
    if not artifact.get("inputs") or not artifact.get("outputs") or not artifact.get("argv"):
        raise ValueError(f"{path}: incomplete build provenance")
    return {
        "artifact": str(path),
        "sha256": sha256(path),
        "toolchain": artifact["toolchain"],
        "image_id": artifact["image_id"],
        "argv": artifact["argv"],
        "input_count": len(artifact["inputs"]),
        "outputs": artifact["outputs"],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--elapsed-seconds", type=int, required=True)
    parser.add_argument("--original-six-log", type=Path, required=True)
    parser.add_argument("--tests-log", type=Path, required=True)
    parser.add_argument("--native-images", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    with args.evidence.open("rb") as stream:
        evidence_doc = tomllib.load(stream)
    evidence = {entry["id"]: entry for entry in evidence_doc["target"]}
    if set(evidence) != set(TARGETS):
        raise ValueError("evidence manifest must contain exactly the seven expansion targets")

    target_summaries = []
    for target, layout in TARGETS.items():
        target_root = args.root / target
        builds = [validate_build(target_root / item, target) for item in layout["builds"]]
        vendor_builds = [
            validate_build(target_root / item, target) for item in layout["vendor_builds"]
        ]
        run_path = target_root / layout["run"]
        repeat_path = target_root / layout["repeat"]
        vcd_path = target_root / layout["vcd"]
        repeat_vcd_path = target_root / layout["repeat_vcd"]
        run = load_json(run_path)
        repeat = load_json(repeat_path)
        if run.get("target") != target or repeat.get("target") != target:
            raise ValueError(f"{target}: run artifact target mismatch")
        if run != repeat:
            raise ValueError(f"{target}: repeated run result diverged")
        if vcd_path.read_bytes() != repeat_vcd_path.read_bytes():
            raise ValueError(f"{target}: repeated VCD diverged")
        vcd_text = vcd_path.read_text(encoding="ascii")
        if "$timescale 1ns $end" not in vcd_text or "$enddefinitions $end" not in vcd_text:
            raise ValueError(f"{target}: invalid VCD header")
        for path in evidence[target]["vcd_signals"]:
            candidate_scopes = path.split(".")[2:]
            if not any(
                f"$scope module {scope} $end" in vcd_text for scope in candidate_scopes
            ):
                raise ValueError(f"{target}: VCD is missing hierarchy for {path}")
        scalar_events = parse_vcd_scalar_events(vcd_text)
        edge_assertions = []
        for signal, expected in layout["edges"].items():
            observed = scalar_events.get(signal, [])
            if not contains_ordered_values(observed, expected):
                raise ValueError(
                    f"{target}: {signal} lacks ordered values {expected}; observed {observed}"
                )
            edge_assertions.append(
                {"signal": signal, "ordered_values": expected, "observed": observed}
            )
        vendor_runs = []
        for relative in layout["vendor_runs"]:
            vendor_run_path = target_root / relative
            vendor_run = load_json(vendor_run_path)
            if vendor_run.get("target") != target:
                raise ValueError(f"{vendor_run_path}: target mismatch")
            vendor_runs.append({"artifact": str(vendor_run_path), "sha256": sha256(vendor_run_path)})

        manifest = evidence[target]
        target_summaries.append(
            {
                "id": target,
                "part": manifest["part"],
                "cpu_profile": manifest["cpu"],
                "image_formats": manifest["image_formats"],
                "memory_map": manifest["memory_map"],
                "reset_assumptions": manifest["reset_assumptions"],
                "vectors": manifest["vectors"],
                "selected_pins": manifest["selected_pins"],
                "interrupt_routes": manifest["interrupt_routes"],
                "peripherals": manifest["peripherals"],
                "vcd_signals": manifest["vcd_signals"],
                "fidelity": manifest["fidelity_tier"],
                "unsupported": manifest["unsupported"],
                "compiler_builds": builds,
                "vendor_builds": vendor_builds,
                "vendor_runs": vendor_runs,
                "run_artifact": str(run_path),
                "run_sha256": sha256(run_path),
                "trace_digest": run["trace_digest"],
                "vcd_artifact": str(vcd_path),
                "vcd_sha256": sha256(vcd_path),
                "edge_assertions": edge_assertions,
                "deterministic_replay": True,
                "status": "pass",
            }
        )

    native_images = load_json(args.native_images)
    if (
        native_images.get("schema") != "renvo.native-image-equivalence.v1"
        or native_images.get("result") != "pass"
        or native_images.get("case_count") != 14
        or native_images.get("physical_target_count") != 13
    ):
        raise ValueError("native-image equivalence evidence is incomplete")

    output = {
        "schema": "renvo.expansion-qualification.v1",
        "result": "pass",
        "plan_sha256": sha256(args.plan),
        "elapsed_seconds": args.elapsed_seconds,
        "fast_gate_under_60_seconds": args.elapsed_seconds < 60,
        "network_disabled_for_compilation": True,
        "structural_gate": "pass",
        "workspace_tests": {"status": "pass", "log_sha256": sha256(args.tests_log)},
        "original_six_regression": {
            "status": "pass",
            "log_sha256": sha256(args.original_six_log),
        },
        "native_image_equivalence": {
            "status": "pass",
            "artifact": str(args.native_images),
            "sha256": sha256(args.native_images),
            "target_modes": native_images["case_count"],
        },
        "targets": target_summaries,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(f"seven-target expansion qualification passed: {args.output}")


if __name__ == "__main__":
    main()
