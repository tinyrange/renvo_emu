#!/usr/bin/env python3
"""Reduce a checkpointed C6 vendor-oracle trace to factual RF candidates."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


UART_DATA = 0x6000_0000
RF_FREQUENCY_CONTROL = 0x600A_00C0
RF_FRONTEND_FORCE = 0x600A_0910
GAIN_TUPLE = (0x600A_08CC, 0x600A_08D0, 0x600A_08D4)
FREQUENCY_CODE_MASK = 0x3FFF
FREQUENCY_START = 1 << 14


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def marker_lines(records: list[dict[str, object]]) -> list[tuple[int, int, str]]:
    lines: list[tuple[int, int, str]] = []
    data = bytearray()
    start: int | None = None
    for record in records:
        if (
            record["region"] != "esp32c6.uart0"
            or record["kind"] != "Write"
            or record["address"] != UART_DATA
        ):
            continue
        if start is None:
            start = int(record["at"])
        byte = int(record["value"]) & 0xFF
        if byte == 10:
            line = data.decode("utf-8", errors="replace").rstrip("\r")
            if "REMU_RF_ORACLE" in line:
                lines.append((start, int(record["at"]), line))
            data.clear()
            start = None
        else:
            data.append(byte)
    return lines


def signed_u32(value: int) -> int:
    return value - (1 << 32) if value & (1 << 31) else value


def gain_triples(records: list[dict[str, object]]) -> list[tuple[int, int, int]]:
    writes = [
        record
        for record in records
        if record["kind"] == "Write" and int(record["address"]) in GAIN_TUPLE
    ]
    triples: list[tuple[int, int, int]] = []
    for index in range(len(writes) - 2):
        group = writes[index : index + 3]
        if tuple(int(item["address"]) for item in group) == GAIN_TUPLE:
            triples.append(tuple(int(item["value"]) for item in group))
    return triples


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--requirements", type=Path, required=True)
    parser.add_argument("--bus", type=Path, required=True)
    parser.add_argument("--replay", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    requirements = json.loads(arguments.requirements.read_text())
    records = json.loads(arguments.bus.read_text())
    replay = json.loads(arguments.replay.read_text())
    observed_regions = {record["region"] for record in records}
    missing_regions = sorted(set(requirements["trace_regions"]) - observed_regions)
    if missing_regions:
        raise SystemExit(f"oracle trace missed required regions: {missing_regions}")
    markers = marker_lines(records)

    begins: dict[str, int] = {}
    ends: dict[str, tuple[int, dict[str, int]]] = {}
    for start, end, line in markers:
        begin = re.search(r"BEGIN stage=(\S+)", line)
        if begin:
            begins[begin.group(1)] = end
        finish = re.search(
            r"END stage=(\S+).*observed_channel=(\d+).*"
            r"observed_power_qdbm=(-?\d+).*tx=(-?\d+)",
            line,
        )
        if finish:
            ends[finish.group(1)] = (
                start,
                {
                    "observed_channel": int(finish.group(2)),
                    "observed_power_qdbm": int(finish.group(3)),
                    "tx_result": int(finish.group(4)),
                },
            )

    replay_frames: list[tuple[str, dict[str, object]]] = []
    for event in replay["events"]:
        if event["event"] != "submitted":
            continue
        request = event["request"]
        frame = request["frame"]
        if frame["protocol"] != "wifi" or len(frame["bytes"]) < 41:
            continue
        ssid = bytes(frame["bytes"][26:41]).decode("ascii", errors="replace")
        match = re.fullmatch(r"REMU-RF-(\d\d)-P(\d\d\d)", ssid)
        if not match:
            continue
        replay_frames.append(
            (
                ssid,
                {
                    "at": request["start"],
                    "center_khz": frame["spectrum"]["center_khz"],
                    "bandwidth_khz": frame["spectrum"]["bandwidth_khz"],
                    "power_dbm": request["power_dbm"],
                    "bytes_sha256": hashlib.sha256(bytes(frame["bytes"])).hexdigest(),
                },
            )
        )

    stage_summaries: list[dict[str, object]] = []
    channel_samples: list[dict[str, object]] = []
    power_samples: list[dict[str, object]] = []
    for expected in requirements["stages"]:
        name = expected["name"]
        if name not in begins or name not in ends:
            raise SystemExit(f"missing bounded checkpoint pair for {name}")
        end_at, observed = ends[name]
        if observed != {
            "observed_channel": expected["channel"],
            "observed_power_qdbm": expected["power_qdbm"],
            "tx_result": 0,
        }:
            raise SystemExit(f"public API outcome mismatch for {name}: {observed}")
        bounded = [
            record
            for record in records
            if begins[name] < int(record["at"]) < end_at
            and record["region"] != "esp32c6.uart0"
        ]
        changed_writes = [
            record
            for record in bounded
            if record["kind"] == "Write"
            and record.get("pre_value") != record.get("post_value")
        ]
        frequency_code = 0x380 + int(expected["channel"]) * 0x280
        frequency_writes = [
            record
            for record in changed_writes
            if int(record["address"]) == RF_FREQUENCY_CONTROL
            and int(record["value"]) & FREQUENCY_START
            and int(record["value"]) & FREQUENCY_CODE_MASK == frequency_code
        ]
        if not frequency_writes:
            raise SystemExit(f"no RFPLL frequency strobe for {name}")
        frontend_values = [
            int(record["value"])
            for record in changed_writes
            if int(record["address"]) == RF_FRONTEND_FORCE
        ]
        if not any(value & 0xF00 == 0x200 for value in frontend_values) or not any(
            value & 0xF00 == 0 for value in frontend_values
        ):
            raise SystemExit(f"no force-off/release pair for {name}")
        triples = gain_triples(bounded)
        if len(triples) < 43:
            raise SystemExit(f"incomplete gain table for {name}: {len(triples)} tuples")
        power_table = triples[-43:]
        if power_table[0][2] != 0xFE:
            raise SystemExit(f"gain table start sentinel changed for {name}")
        decoded_power = signed_u32(power_table[-1][2]) // 128 + 133
        if decoded_power != expected["power_qdbm"]:
            raise SystemExit(
                f"gain table for {name} decodes {decoded_power}, expected {expected['power_qdbm']}"
            )
        ssid = f"REMU-RF-{expected['channel']:02d}-P{expected['power_qdbm']:03d}"
        matching_submissions = [
            frame
            for observed_ssid, frame in replay_frames
            if observed_ssid == ssid and begins[name] < int(frame["at"]) < end_at
        ]
        if len(matching_submissions) != 1:
            raise SystemExit(f"missing tagged RF submission {ssid}")
        expected_airtime = {
            "center_khz": (
                2_484_000
                if expected["channel"] == 14
                else 2_412_000 + (int(expected["channel"]) - 1) * 5_000
            ),
            "bandwidth_khz": 20_000,
            "power_dbm": int(expected["power_qdbm"]) // 4,
        }
        observed_airtime = matching_submissions[0]
        for field, value in expected_airtime.items():
            if observed_airtime[field] != value:
                raise SystemExit(
                    f"causal airtime mismatch for {name}: {field}="
                    f"{observed_airtime[field]}, expected {value}"
                )
        stage_summaries.append(
            {
                "name": name,
                "start": begins[name],
                "end": end_at,
                **observed,
                "changed_writes": len(changed_writes),
                "gain_tuples": len(triples),
                "tagged_submission": matching_submissions[0],
            }
        )
        if name in {"CHANNEL_1", "CHANNEL_6", "CHANNEL_11"}:
            channel_samples.append(
                {
                    "channel": expected["channel"],
                    "frequency_code": frequency_code,
                    "write_values": sorted({int(item["value"]) for item in frequency_writes}),
                    "pcs": sorted({int(item["pc"]) for item in frequency_writes}),
                }
            )
        if name in {"POWER_LOW", "POWER_MEDIUM", "POWER_HIGH"}:
            power_samples.append(
                {
                    "requested_qdbm": expected["power_qdbm"],
                    "final_gain_word": power_table[-1][2],
                    "decoded_qdbm": decoded_power,
                }
            )

    required_markers = {
        "REMU_RF_ORACLE COLD_INIT_BEGIN",
        "REMU_RF_ORACLE COLD_INIT_END",
        "REMU_RF_ORACLE WARM_DISABLE_BEGIN",
        "REMU_RF_ORACLE WARM_DISABLE_END",
        "REMU_RF_ORACLE WARM_ENABLE_BEGIN",
        "REMU_RF_ORACLE WARM_ENABLE_END",
        "REMU_RF_ORACLE RADIO_RESET_BEGIN",
        "REMU_RF_ORACLE RADIO_RESET_DEINITIALIZED",
        "REMU_RF_ORACLE RADIO_RESET_END",
        "REMU_RF_ORACLE DONE",
    }
    observed_markers = {line for _, _, line in markers}
    missing = sorted(required_markers - observed_markers)
    if missing:
        raise SystemExit(f"missing lifecycle markers: {missing}")

    output = {
        "schema": "remu.c6-rf-oracle-analysis.v1",
        "chip": "esp32c6",
        "provenance": {
            "source_ledger_entries": requirements["source_ledger_entries"],
            "pc_is_observational_only": True,
            "symbol_dispatch_allowed": False,
            "bus_sha256": digest(arguments.bus),
            "radio_replay_sha256": digest(arguments.replay),
        },
        "trace": {
            "records": len(records),
            "regions": sorted(observed_regions),
            "all_records_have_pc": all("pc" in record for record in records),
            "rf_writes_with_safe_pre_post": sum(
                1
                for record in records
                if record["kind"] == "Write"
                and record["region"] != "esp32c6.uart0"
                and "pre_value" in record
                and "post_value" in record
            ),
        },
        "stages": stage_summaries,
        "recovered_candidates": {
            "rfpll_channel": {
                "classification": "firmware-observed-primary-object-supported",
                "confidence": "high",
                "address": RF_FREQUENCY_CONTROL,
                "start_mask": FREQUENCY_START,
                "code_mask": FREQUENCY_CODE_MASK,
                "code_formula": "0x380 + channel * 0x280",
                "samples": channel_samples,
            },
            "tx_power_table": {
                "classification": "firmware-observed-primary-object-supported",
                "confidence": "high",
                "tuple_addresses": list(GAIN_TUPLE),
                "entries": 43,
                "final_word_formula": "signed_u32(word) / 128 + 133 quarter-dBm",
                "samples": power_samples,
            },
            "frontend_force": {
                "classification": "firmware-observed-primary-object-supported",
                "confidence": "high",
                "address": RF_FRONTEND_FORCE,
                "field_mask": 0xF00,
                "forced_off": 0x200,
                "released": 0,
            },
        },
    }
    arguments.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
