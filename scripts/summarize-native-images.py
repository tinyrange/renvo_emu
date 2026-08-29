#!/usr/bin/env python3
"""Validate native/direct equivalence artifacts and emit stable evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


CASES = [
    ("ch32v003", "ch32v003", "elf", "raw-bin", "wch"),
    ("ch32v006", "ch32v006", "elf", "raw-bin", "wch"),
    ("rp2040", "rp2040", "elf", "uf2", "rp2040"),
    ("rp2350-arm", "rp2350", "elf", "uf2", "rp2350-arm"),
    ("rp2350-riscv", "rp2350", "elf", "uf2", "rp2350-riscv"),
    ("esp32s3", "esp32s3", "elf", "esp-bin", "esp32s3"),
    ("esp32c6", "esp32c6", "elf", "esp-bin", "esp32c6"),
    ("atsamd21e18", "atsamd21e18", "elf", "raw-bin", "atsamd21e18"),
    ("stm32l432kc", "stm32l432kc", "elf", "raw-bin", "stm32l432kc"),
    ("stm32f411re", "stm32f411re", "elf", "raw-bin", "stm32f411re"),
    ("nrf52840", "nrf52840", "elf", "raw-bin", "nrf52840"),
    ("esp32p4", "esp32p4", "elf", "raw-bin", "esp32p4"),
    ("r7fa4m1ab3cfm", "r7fa4m1ab3cfm", "elf", "raw-bin", "r7fa4m1ab3cfm"),
    ("atmega328pb", "atmega328pb", "elf", "intel-hex", "atmega328pb"),
    ("msp430fr2433", "msp430fr2433", "elf", "intel-hex", "msp430fr2433"),
    ("pic16f15376", "pic16f15376", "intel-hex", "intel-hex", "pic16f15376"),
    ("efm8bb52f32g", "efm8bb52f32g", "intel-hex", "intel-hex", "efm8bb52f32g"),
]

OBSERVABLE_FIELDS = ("reason", "exit_code", "uart", "usb", "trace_digest")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict:
    with path.open("rb") as stream:
        return json.load(stream)


def observable(result: dict) -> dict:
    return {field: result.get(field) for field in OBSERVABLE_FIELDS}


def is_immutable_image_id(value: object) -> bool:
    return (
        isinstance(value, str)
        and value.startswith("sha256:")
        and len(value) == 71
        and all(character in "0123456789abcdef" for character in value[7:])
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source", type=Path, action="append", default=[])
    args = parser.parse_args()

    cases = []
    for case_id, target, direct_format, native_format, build_id in CASES:
        run_root = args.root / "run" / case_id
        direct_path = run_root / "direct.json"
        native_path = run_root / "native.json"
        direct_vcd = run_root / "direct.vcd"
        native_vcd = run_root / "native.vcd"
        build_path = args.root / "build" / f"{build_id}.json"
        direct = load_json(direct_path)
        native = load_json(native_path)
        build = load_json(build_path)
        if direct.get("target") != target or native.get("target") != target:
            raise ValueError(f"{case_id}: target mismatch")
        if observable(direct) != observable(native):
            raise ValueError(f"{case_id}: native and direct observations diverged")
        if direct.get("exit_code") != 0 or native.get("exit_code") != 0:
            raise ValueError(f"{case_id}: probe did not report exit code zero")
        if direct_vcd.read_bytes() != native_vcd.read_bytes():
            raise ValueError(f"{case_id}: native and direct VCD output diverged")
        if build.get("schema") != "remu.build-artifact.v1":
            raise ValueError(f"{build_path}: invalid build artifact")
        if build.get("exit_code") != 0 or build.get("timed_out"):
            raise ValueError(f"{build_path}: compilation failed")
        # DockerCompiler resolves either the recorded digest or the explicitly
        # allowed local bootstrap tag to an ID before launching the container.
        if not is_immutable_image_id(build.get("image_id")):
            raise ValueError(f"{build_path}: toolchain did not resolve to an immutable image ID")
        cases.append(
            {
                "id": case_id,
                "target": target,
                "direct_format": direct_format,
                "native_format": native_format,
                "toolchain": build["toolchain"],
                "toolchain_image": build["image_id"],
                "build_artifact": str(build_path),
                "build_sha256": sha256(build_path),
                "direct_result": str(direct_path),
                "direct_sha256": sha256(direct_path),
                "native_result": str(native_path),
                "native_sha256": sha256(native_path),
                "vcd_sha256": sha256(direct_vcd),
                "trace_digest": direct["trace_digest"],
                "status": "pass",
            }
        )

    rejection_root = args.root / "rejections"
    rejections = []
    for path in sorted(rejection_root.glob("*.log")):
        if not path.read_text(encoding="utf-8").strip():
            raise ValueError(f"{path}: rejection produced no diagnostic")
        rejections.append({"id": path.stem, "artifact": str(path), "sha256": sha256(path)})
    if len(rejections) < 4:
        raise ValueError("native-image qualification requires at least four rejection cases")

    source_digest = hashlib.sha256()
    for path in sorted(args.source):
        source_digest.update(str(path).encode("utf-8"))
        source_digest.update(path.read_bytes())
    output = {
        "schema": "remu.native-image-equivalence.v1",
        "result": "pass",
        "source_sha256": source_digest.hexdigest(),
        "docker_compilation": True,
        "network_disabled_for_compilation": True,
        "case_count": len(cases),
        "physical_target_count": len({case["target"] for case in cases}),
        "observable_fields": list(OBSERVABLE_FIELDS),
        "cases": cases,
        "negative_cases": rejections,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(f"native/direct equivalence passed for {len(cases)} target modes: {args.output}")


if __name__ == "__main__":
    main()
