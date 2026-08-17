#!/usr/bin/env python3
"""Enforce the independent ESP32-C6 radio driver's declared MMIO surface."""

from __future__ import annotations

import json
import re
from collections import defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DRIVER = ROOT / "qualification/radio/c6-rf-probe"
MANIFEST = DRIVER / "mmio-contract.json"
REFERENCE = ROOT / "qualification/radio/c6-rf-register-reference.json"
HEADER = DRIVER / "hal/c6/registers.h"
ACCESS = {
    "read": {"read"},
    "write": {"write"},
    "read-write": {"read", "write"},
}
EVIDENCE_TIERS = {"trace-causal", "emulator-contract", "platform-contract"}
RADIO_DOMAINS = {"radio-rf", "radio-mac", "radio-control"}
REQUIRED_FIELDS = {
    "symbol", "address", "span_bytes", "stride_bytes", "access", "domain",
    "evidence_tier", "lifecycle", "technical_function", "value_constraints",
    "side_effect", "failure_mode", "provenance",
}


def fail(message: str) -> None:
    raise SystemExit(f"C6 custom-driver contract check failed: {message}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def sanitize_c(source: str) -> str:
    """Remove comments and literals while retaining syntax and line structure."""
    pattern = re.compile(r"/\*.*?\*/|//[^\n]*|\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*'", re.S)
    return pattern.sub(lambda match: "\n" * match.group(0).count("\n"), source)


def mmio_calls(source: str) -> list[tuple[str, str]]:
    """Return (direction, first argument) for balanced c6_read/write32 calls."""
    calls = []
    for match in re.finditer(r"\bc6_(read|write)32\s*\(", source):
        direction = match.group(1)
        start = match.end()
        depth = 1
        cursor = start
        while cursor < len(source) and depth:
            character = source[cursor]
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    calls.append((direction, source[start:cursor].strip()))
                    break
            elif character == "," and depth == 1:
                calls.append((direction, source[start:cursor].strip()))
                break
            cursor += 1
        else:
            fail("unbalanced c6_read32/c6_write32 call")
    return calls


def peripheral_literal(value: int) -> bool:
    return 0x60000000 <= value <= 0x600FFFFF or 0x20000000 <= value <= 0x200FFFFF


def declared_offset(argument: str, symbol: str) -> int:
    """Resolve an exact symbol or symbol-plus-literal bounded-region access."""
    match = re.fullmatch(
        rf"\s*{re.escape(symbol)}(?:\s*\+\s*(0x[0-9a-fA-F]+|[0-9]+)[uUlL]*)?\s*",
        argument,
    )
    require(match is not None, f"{symbol} target must use a fixed manifest-bounded offset: {argument}")
    return int(match.group(1), 0) if match.group(1) else 0


def parse_header() -> dict[str, int]:
    definitions = {}
    expression = re.compile(r"^\s*#define\s+(C6_REG_[A-Z0-9_]+)\s+(0x[0-9a-fA-F]+)[uUlL]*\s*$")
    for line_number, line in enumerate(HEADER.read_text().splitlines(), 1):
        if "#define C6_REG_" not in line:
            continue
        match = expression.match(line)
        require(match is not None, f"{HEADER.relative_to(ROOT)}:{line_number} must use one literal address")
        symbol, literal = match.groups()
        require(symbol not in definitions, f"duplicate address symbol {symbol}")
        definitions[symbol] = int(literal, 16)
    return definitions


def parser_self_test() -> None:
    sample = "c6_write32(C6_REG_TABLE + 8u + index * 4u, value); c6_read32(C6_REG_STATUS)"
    assert mmio_calls(sample) == [
        ("write", "C6_REG_TABLE + 8u + index * 4u"),
        ("read", "C6_REG_STATUS"),
    ]
    assert not mmio_calls(sanitize_c("/* c6_read32(C6_REG_FAKE) */"))
    assert declared_offset("C6_REG_TABLE", "C6_REG_TABLE") == 0
    assert declared_offset("C6_REG_TABLE + 36u", "C6_REG_TABLE") == 36


def main() -> None:
    parser_self_test()
    manifest = json.loads(MANIFEST.read_text())
    reference = json.loads(REFERENCE.read_text())
    require(manifest["schema"] == "remu.custom-radio-driver-mmio.v1", "manifest schema")
    require(manifest["driver"] == "c6-rf-probe", "driver identity")
    require(manifest["target"] == "esp32c6", "target identity")
    require(manifest["scope"] == "software-emulator-only", "scope must remain emulator-only")
    require(manifest["policy"]["physical_rf_execution"] == "forbidden", "physical RF policy")
    require(manifest["policy"]["address_header"] == str(HEADER.relative_to(ROOT)), "address-header policy path")
    require(manifest["policy"]["radio_reference"] == str(REFERENCE.relative_to(ROOT)), "radio-reference policy path")

    definitions = parse_header()
    entries = manifest["registers"]
    symbols = [entry["symbol"] for entry in entries]
    require(len(symbols) == len(set(symbols)), "duplicate manifest symbol")
    require(set(symbols) == set(definitions), "manifest/header symbol sets differ")
    lifecycle = set(manifest["lifecycle_order"])

    occupied: dict[int, str] = {}
    entry_by_symbol = {}
    for entry in entries:
        missing = REQUIRED_FIELDS - set(entry)
        require(not missing, f"{entry.get('symbol', '<unnamed>')} missing fields {sorted(missing)}")
        symbol = entry["symbol"]
        entry_by_symbol[symbol] = entry
        address = int(entry["address"], 16)
        span = entry["span_bytes"]
        stride = entry["stride_bytes"]
        require(definitions[symbol] == address, f"{symbol} header/manifest address mismatch")
        require(address % 4 == 0, f"{symbol} is not word aligned")
        require(isinstance(span, int) and span >= 4 and span % 4 == 0, f"{symbol} invalid span")
        require(isinstance(stride, int) and stride >= 4 and stride % 4 == 0, f"{symbol} invalid stride")
        require(span % stride == 0, f"{symbol} span is not a multiple of stride")
        require(entry["access"] in ACCESS, f"{symbol} invalid access policy")
        require(entry["evidence_tier"] in EVIDENCE_TIERS, f"{symbol} invalid evidence tier")
        require(entry["lifecycle"] and set(entry["lifecycle"]) <= lifecycle, f"{symbol} invalid lifecycle")
        for field in REQUIRED_FIELDS - {"span_bytes", "stride_bytes", "lifecycle"}:
            require(isinstance(entry[field], str) and entry[field].strip(), f"{symbol} empty {field}")
        for offset in range(0, span, stride):
            word = address + offset
            require(word not in occupied, f"{symbol} overlaps {occupied.get(word)} at {word:#x}")
            occupied[word] = symbol

    reference_by_address = {int(item["address"], 16): item for item in reference["registers"]}
    for entry in entries:
        if entry["domain"] not in RADIO_DOMAINS:
            require(entry["evidence_tier"] == "platform-contract", f"{entry['symbol']} non-radio evidence tier")
            continue
        require(entry["evidence_tier"] != "platform-contract", f"{entry['symbol']} radio register lacks radio evidence")
        address = int(entry["address"], 16)
        for offset in range(0, entry["span_bytes"], entry["stride_bytes"]):
            item = reference_by_address.get(address + offset)
            require(item is not None, f"{entry['symbol']} missing from C6 RF reference at {address + offset:#x}")
            require(item["semantically_implemented"], f"{entry['symbol']} depends on trace-only unknown {address + offset:#x}")
            require(item["confidence"] in {"high", "medium"}, f"{entry['symbol']} has insufficient confidence")
            if entry["evidence_tier"] == "trace-causal":
                require(item["trace_observed"], f"{entry['symbol']} claims unobserved trace evidence at {address + offset:#x}")

    used: dict[str, set[str]] = defaultdict(set)
    symbol_uses: dict[str, int] = defaultdict(int)
    call_uses: dict[str, int] = defaultdict(int)
    source_files = sorted([*DRIVER.rglob("*.c"), *DRIVER.rglob("*.h")])
    for path in source_files:
        source = sanitize_c(path.read_text())
        if path != HEADER:
            for symbol in re.findall(r"\bC6_REG_[A-Z0-9_]+\b", source):
                symbol_uses[symbol] += 1
            for literal in re.finditer(r"\b0x[0-9a-fA-F]+[uUlL]*\b", source):
                value = int(re.match(r"0x[0-9a-fA-F]+", literal.group(0)).group(0), 16)
                require(not peripheral_literal(value), f"raw peripheral address {value:#x} in {path.relative_to(ROOT)}")
        if path.name == "mmio.h" or path == HEADER:
            continue
        if path.suffix == ".c" and path.parent == DRIVER / "hal/c6":
            require(
                re.search(r"\breturn\s+-\s*[0-9]+\s*;", source) is None,
                f"numeric negative return in {path.relative_to(ROOT)}; declare a named subsystem result",
            )
            require(
                re.search(r"\(\s*volatile\s+[A-Za-z_][A-Za-z0-9_ ]*\*\s*\)", source) is None,
                f"direct volatile pointer cast in {path.relative_to(ROOT)}; use the audited MMIO primitive",
            )
        for direction, argument in mmio_calls(source):
            roots = set(re.findall(r"\bC6_REG_[A-Z0-9_]+\b", argument))
            require(len(roots) == 1, f"{path.relative_to(ROOT)} {direction} target must contain exactly one declared C6_REG symbol: {argument}")
            symbol = roots.pop()
            require(symbol in entry_by_symbol, f"{path.relative_to(ROOT)} uses undeclared {symbol}")
            require(direction in ACCESS[entry_by_symbol[symbol]["access"]], f"{path.relative_to(ROOT)} {direction} exceeds {symbol} access policy")
            offset = declared_offset(argument, symbol)
            require(offset < entry_by_symbol[symbol]["span_bytes"], f"{path.relative_to(ROOT)} {symbol} offset {offset} exceeds declared span")
            require(offset % entry_by_symbol[symbol]["stride_bytes"] == 0, f"{path.relative_to(ROOT)} {symbol} offset {offset} violates stride")
            used[symbol].add(direction)
            call_uses[symbol] += 1

    require(set(used) == set(entry_by_symbol), f"unused declarations: {sorted(set(entry_by_symbol) - set(used))}")
    require(symbol_uses == call_uses, "register symbols may appear only as audited MMIO call targets")
    for symbol, directions in used.items():
        require(directions == ACCESS[entry_by_symbol[symbol]["access"]], f"{symbol} manifest access is broader than actual use")

    radio_count = sum(entry["domain"] in RADIO_DOMAINS for entry in entries)
    range_count = sum(entry["span_bytes"] > 4 for entry in entries)
    print(
        f"C6 custom-driver contract: {len(entries)} dependencies / {radio_count} radio / "
        f"{range_count} bounded ranges; all MMIO calls declared"
    )


if __name__ == "__main__":
    main()
