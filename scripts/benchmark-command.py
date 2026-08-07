#!/usr/bin/env python3
"""Run one bounded command and record reproducible host-performance metrics."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import resource
import subprocess
import sys
import time
from pathlib import Path
from typing import Iterable


SCHEMA = "remu.benchmark-command.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def host_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
            if line.lower().startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or platform.machine()


def rss_bytes() -> int:
    usage = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    # Linux and the BSDs report KiB; macOS reports bytes.
    return int(usage if sys.platform == "darwin" else usage * 1024)


def artifact_records(paths: Iterable[Path]) -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for path in paths:
        record: dict[str, object] = {"path": str(path)}
        if path.is_file():
            record["bytes"] = path.stat().st_size
            record["sha256"] = sha256(path)
        else:
            record["bytes"] = 0
            record["missing"] = True
        records.append(record)
    return records


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", default="benchmark")
    parser.add_argument("--artifact", action="append", type=Path, default=[])
    parser.add_argument("--actions", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    arguments = parser.parse_args()
    command = list(arguments.command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("a command is required after --")

    output = arguments.output
    output.parent.mkdir(parents=True, exist_ok=True)
    stdout_path = output.with_suffix(output.suffix + ".stdout")
    stderr_path = output.with_suffix(output.suffix + ".stderr")
    started = time.perf_counter_ns()
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        return_code = process.wait()
    elapsed_ns = time.perf_counter_ns() - started
    actions_per_second = None
    if arguments.actions is not None and arguments.actions >= 0:
        actions_per_second = (
            arguments.actions / (elapsed_ns / 1_000_000_000)
            if elapsed_ns
            else 0.0
        )
    record = {
        "schema": SCHEMA,
        "label": arguments.label,
        "command": command,
        "command_sha256": hashlib.sha256(
            json.dumps(command, separators=(",", ":")).encode("utf-8")
        ).hexdigest(),
        "status": "pass" if return_code == 0 else "fail",
        "exit_code": return_code,
        "wall_time_ns": elapsed_ns,
        "wall_time_seconds": elapsed_ns / 1_000_000_000,
        "peak_rss_bytes": rss_bytes(),
        "actions": arguments.actions,
        "actions_per_second": actions_per_second,
        "artifacts": artifact_records(arguments.artifact),
        "stdout": {"path": str(stdout_path), "sha256": sha256(stdout_path)},
        "stderr": {"path": str(stderr_path), "sha256": sha256(stderr_path)},
        "host": {
            "model": host_model(),
            "architecture": platform.machine(),
            "kernel": platform.release(),
            "system": platform.system(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count(),
        },
    }
    output.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if return_code:
        print(
            f"benchmark command failed ({arguments.label}); see {stderr_path}",
            file=sys.stderr,
        )
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
