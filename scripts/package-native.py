#!/usr/bin/env python3
"""Deterministically package raw flash bytes for native-image qualification."""

from __future__ import annotations

import argparse
from pathlib import Path
import struct


UF2_MAGIC_START_0 = 0x0A324655
UF2_MAGIC_START_1 = 0x9E5D5157
UF2_MAGIC_END = 0x0AB16F30
UF2_FLAG_FAMILY_ID = 0x00002000
UF2_PAYLOAD = 256


def pack_uf2(source: Path, output: Path, address: int, family: int) -> None:
    payload = source.read_bytes()
    blocks = (len(payload) + UF2_PAYLOAD - 1) // UF2_PAYLOAD
    encoded = bytearray()
    for index in range(blocks):
        chunk = payload[index * UF2_PAYLOAD : (index + 1) * UF2_PAYLOAD]
        block = bytearray(512)
        struct.pack_into(
            "<IIIIIIII",
            block,
            0,
            UF2_MAGIC_START_0,
            UF2_MAGIC_START_1,
            UF2_FLAG_FAMILY_ID,
            address + index * UF2_PAYLOAD,
            len(chunk),
            index,
            blocks,
            family,
        )
        block[32 : 32 + len(chunk)] = chunk
        struct.pack_into("<I", block, 508, UF2_MAGIC_END)
        encoded.extend(block)
    output.write_bytes(encoded)


def overlay(base: Path, application: Path, output: Path, offset: int) -> None:
    merged = bytearray(base.read_bytes())
    payload = application.read_bytes()
    end = offset + len(payload)
    if end > len(merged):
        merged.extend(b"\xff" * (end - len(merged)))
    merged[offset:end] = payload
    output.write_bytes(merged)


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    uf2 = subparsers.add_parser("uf2")
    uf2.add_argument("--input", type=Path, required=True)
    uf2.add_argument("--output", type=Path, required=True)
    uf2.add_argument("--address", type=lambda value: int(value, 0), required=True)
    uf2.add_argument("--family", type=lambda value: int(value, 0), required=True)

    merge = subparsers.add_parser("overlay")
    merge.add_argument("--base", type=Path, required=True)
    merge.add_argument("--application", type=Path, required=True)
    merge.add_argument("--output", type=Path, required=True)
    merge.add_argument("--offset", type=lambda value: int(value, 0), required=True)

    args = parser.parse_args()
    if args.command == "uf2":
        pack_uf2(args.input, args.output, args.address, args.family)
    else:
        overlay(args.base, args.application, args.output, args.offset)


if __name__ == "__main__":
    main()
