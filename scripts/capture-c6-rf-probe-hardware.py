#!/usr/bin/env python3
"""Capture and validate c6-rf-probe checkpoints from a real serial device."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import termios
import time


REQUIRED = ["INIT", "CH1-P08", "CH6-P14", "CH11-P20", "WARM-P14",
            "RESET_RADIO", "RESET-P14", "READY"]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=15.0)
    args = parser.parse_args()

    flags = os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK
    descriptor = os.open(args.port, flags)
    try:
        attributes = termios.tcgetattr(descriptor)
        attributes[0] = 0
        attributes[1] = 0
        attributes[2] = termios.CS8 | termios.CLOCAL | termios.CREAD
        attributes[3] = 0
        attributes[4] = termios.B115200
        attributes[5] = termios.B115200
        termios.tcsetattr(descriptor, termios.TCSANOW, attributes)
        deadline = time.monotonic() + args.seconds
        captured = bytearray()
        sent_probe = False
        while time.monotonic() < deadline:
            ready, _, _ = select.select([descriptor], [], [], 0.25)
            if ready:
                try:
                    captured.extend(os.read(descriptor, 4096))
                except BlockingIOError:
                    pass
            if not sent_probe and b"event=READY result=0" in captured:
                os.write(descriptor, b"CHANNEL 1\nPOWER 8\nTX HW-C1-P08\n"
                                     b"CHANNEL 6\nPOWER 14\nRX START\n")
                sent_probe = True
        text = captured.decode("utf-8", errors="replace")
    finally:
        os.close(descriptor)

    missing = [event for event in REQUIRED
               if f"event={event} result=0" not in text]
    if missing:
        raise SystemExit(f"physical C6 omitted checkpoints: {missing}")
    if "event=TX result=0 channel=1 power_dbm=8" not in text:
        raise SystemExit("physical C6 did not execute the UART TX command")
    result = {
        "schema": "remu.c6-rf-probe-hardware.v1",
        "device": str(args.port.resolve()),
        "exact_flash_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
        "capture_unix_ns": time.time_ns(),
        "required_checkpoints": "pass",
        "uart_command_protocol": "pass",
        "uart": text,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
