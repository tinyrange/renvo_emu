#!/usr/bin/env python3
"""Minimal standards-level GDB RSP client used by the Renvo Emulator qualification."""

import json
import socket
import sys


def checksum(payload: bytes) -> bytes:
    return f"{sum(payload) & 0xff:02x}".encode("ascii")


def packet(sock: socket.socket, payload: str) -> str:
    encoded = payload.encode("ascii")
    sock.sendall(b"$" + encoded + b"#" + checksum(encoded))
    acknowledgment = sock.recv(1)
    if acknowledgment != b"+":
        raise RuntimeError(f"RSP request was not acknowledged: {acknowledgment!r}")
    marker = sock.recv(1)
    if marker != b"$":
        raise RuntimeError(f"missing RSP response marker: {marker!r}")
    response = bytearray()
    while True:
        byte = sock.recv(1)
        if byte == b"#":
            break
        if not byte:
            raise RuntimeError("RSP server disconnected inside a packet")
        response.extend(byte)
    received_checksum = sock.recv(2)
    if received_checksum != checksum(response):
        raise RuntimeError("RSP response checksum mismatch")
    sock.sendall(b"+")
    return response.decode("utf-8")


def main() -> None:
    address, entry_text, expected_architecture, transcript_path = sys.argv[1:]
    host, port_text = address.rsplit(":", 1)
    entry = int(entry_text, 0)
    code_address = entry & ~1 if expected_architecture == "arm" else entry
    transcript = []
    with socket.create_connection((host, int(port_text)), timeout=10) as sock:
        def check(request: str, predicate) -> str:
            response = packet(sock, request)
            transcript.append({"request": request, "response": response})
            if not predicate(response):
                raise RuntimeError(f"unexpected response to {request!r}: {response!r}")
            return response

        check("?", lambda value: value == "S05")
        check("qSupported:multiprocess+", lambda value: "qXfer:features:read+" in value)
        xml = check(
            "qXfer:features:read:target.xml:0,1000",
            lambda value: value.startswith("l") and expected_architecture in value,
        )
        check("g", lambda value: len(value) >= 8 and not value.startswith("E"))
        check("p0", lambda value: len(value) == 8)
        check(f"m{code_address:x},4", lambda value: len(value) == 8)
        check(f"Z0,{code_address:x},2", lambda value: value == "OK")
        check("c", lambda value: value == "S05")
        check(f"z0,{code_address:x},2", lambda value: value == "OK")
        check("s", lambda value: value == "S05")
        check("D", lambda value: value == "OK")

    with open(transcript_path, "w", encoding="utf-8") as output:
        json.dump(
            {
                "schema": "remu.gdb-client-transcript.v1",
                "architecture": expected_architecture,
                "target_xml_size": len(xml) - 1,
                "packets": transcript,
                "result": "pass",
            },
            output,
            indent=2,
            sort_keys=True,
        )
        output.write("\n")


if __name__ == "__main__":
    main()
