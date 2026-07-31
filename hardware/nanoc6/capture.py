#!/usr/bin/env python3
"""Capture one complete Renvo result frame from an ESP USB serial console."""

from __future__ import annotations

import argparse
import sys
import time

import serial


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("port")
    parser.add_argument("--timeout", type=float, default=15.0)
    arguments = parser.parse_args()

    deadline = time.monotonic() + arguments.timeout
    in_frame = False
    console = serial.Serial()
    console.port = arguments.port
    console.baudrate = 115200
    console.timeout = 0.25
    console.dtr = False
    console.rts = False
    console.open()
    with console:
        while time.monotonic() < deadline:
            raw = console.readline()
            if not raw:
                continue
            line = raw.decode("utf-8", errors="replace").strip()
            if line.startswith("RENVO_HW_BEGIN "):
                in_frame = True
            if in_frame:
                print(line, flush=True)
            if in_frame and line.startswith("RENVO_HW_END "):
                return 0

    print("timed out waiting for a complete RENVO_HW frame", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
