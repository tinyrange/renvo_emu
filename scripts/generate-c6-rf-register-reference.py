#!/usr/bin/env python3
"""Generate the factual ESP32-C6 RF register/value reference from a bus trace."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path


BASES = {
    "esp32c6.power-detector": 0x600A0000,
    "esp32c6.ble-baseband-registers": 0x600A1000,
    "esp32c6.phy-mac-registers": 0x600A2000,
    "esp32c6.ieee802154": 0x600A3000,
    "esp32c6.wifi-mac-registers": 0x600A4000,
    "esp32c6.phy-baseband-registers": 0x600A7000,
    "esp32c6.phy-front-end-registers": 0x600A8000,
    "esp32c6.ble-control-registers": 0x600A9000,
    "esp32c6.modem-syscon": 0x600A9800,
    "esp32c6.phy-registers": 0x600AD000,
    "esp32c6.ble-modem-registers": 0x600AE000,
    "esp32c6.modem-lpcon": 0x600AF000,
    "esp32c6.i2c-ana-mst": 0x600AF800,
    "esp32c6.phy-i2c-command-memory": 0x600AFC00,
}

# Only assign a function where repository behavior and bounded trace evidence
# establish it. Every other observed address remains explicitly unknown.
KNOWN: dict[int, tuple[str, str, str, int | None, int | None]] = {
    0x600A00C0: ("RFPLL_CHANNEL_CONTROL", "RFPLL mode/channel code and start strobe", "high"),
    0x600A00CC: ("RFPLL_CHANNEL_STATUS", "RFPLL completion status", "medium"),
    0x600A0418: ("POWER_DETECTOR_CONVERSION", "start and synchronously complete RF power conversion", "high"),
    0x600A0474: ("IQ_ESTIMATE_CONTROL", "IQ calibration start strobe", "high"),
    0x600A04A0: ("IQ_ESTIMATE_STATUS", "IQ calibration completion status", "medium"),
    0x600A0810: ("TX_TONE_CONTROL", "power-detector tone control", "medium"),
    0x600A0814: ("TX_TONE_STATUS", "power-detector tone status", "medium"),
    0x600A08CC: ("TX_GAIN_FIRST", "first word of a 43-entry TX gain tuple", "high"),
    0x600A08D0: ("TX_GAIN_SECOND", "second word of a 43-entry TX gain tuple", "high"),
    0x600A08D4: ("TX_GAIN_FINAL", "final word; completes tuple and encodes power ceiling", "high"),
    0x600A0910: ("RF_FRONTEND_FORCE", "force-off/release state for Wi-Fi frontend", "high"),
    0x600A405C: ("WIFI_INTERFACE0_LOW", "station interface address bytes 0..3", "high"),
    0x600A4060: ("WIFI_INTERFACE0_HIGH", "station address bytes 4..5 and valid bit", "high"),
    0x600A4080: ("WIFI_RX_CONTROL", "RX descriptor reload command", "high"),
    0x600A4084: ("WIFI_RX_BASE", "firmware-owned RX descriptor base", "high"),
    0x600A4814: ("WIFI_CRYPTO_VALID", "valid bitmap for 32 native key slots", "high"),
    0x600A4C40: ("WIFI_INTERRUPT_MASK", "Wi-Fi MAC interrupt mask", "high"),
    0x600A4C48: ("WIFI_INTERRUPT_EVENT", "latched Wi-Fi MAC interrupt events", "high"),
    0x600A4C4C: ("WIFI_INTERRUPT_CLEAR", "write-one-to-clear Wi-Fi events", "high"),
    0x600A4CB4: ("WIFI_TX_QUEUE_STATE_CLEAR", "clear completed TX queue bits", "high"),
    0x600A4CB8: ("WIFI_TX_QUEUE_STATE", "completed TX queue bitmap", "high"),
    0x600A4D6C: ("WIFI_TX_QUEUE0_CONTROL", "queue-0 enable and descriptor pointer", "high"),
    0x600A4DDC: ("WIFI_RESET_CONTROL", "Wi-Fi reset strobe and ready acknowledgement", "high"),
    0x600A5800: ("WIFI_CRYPTO_SLOT0", "first word of native crypto slot 0", "high"),
    0x600A9814: ("MODEM_CLOCK_ENABLE", "Wi-Fi/BLE/802.15.4 clock-domain gates", "high"),
}


def modeled(
    address: int,
    name: str,
    function: str,
    confidence: str = "high",
    writable_mask: int | None = None,
    reset_value: int | None = 0,
) -> None:
    """Add one register whose behavior is explicitly modeled in repository code."""
    previous = KNOWN.get(address)
    if previous is not None and len(previous) == 5:
        KNOWN[address] = (
            previous[0],
            previous[1],
            previous[2],
            previous[3] if previous[3] is not None else writable_mask,
            previous[4] if previous[4] is not None else reset_value,
        )
        return
    KNOWN[address] = (name, function, confidence, writable_mask, reset_value)


# Normalize the compact causal table above, then enumerate every additional
# register with semantic behavior in the C6 radio devices. Generic backing-array
# words are not called implemented unless the oracle observed them.
for address, entry in list(KNOWN.items()):
    if len(entry) == 3:
        KNOWN[address] = (*entry, None, 0)

# Wi-Fi MAC interfaces, DMA, queues, block-ack state, and native crypto table.
wifi = BASES["esp32c6.wifi-mac-registers"]
for interface in range(4):
    modeled(wifi + 0x05C + interface * 8, f"WIFI_INTERFACE{interface}_LOW", "interface MAC address bytes 0..3")
    modeled(wifi + 0x060 + interface * 8, f"WIFI_INTERFACE{interface}_HIGH", "interface MAC address bytes 4..5 and valid bit")
for offset, name, function in [
    (0x080, "WIFI_RX_CONTROL", "RX descriptor reload command"),
    (0x084, "WIFI_RX_BASE", "RX DMA descriptor base"),
    (0x088, "WIFI_RX_NEXT", "next RX DMA descriptor selected by the model"),
    (0x08C, "WIFI_RX_LAST", "last completed RX DMA descriptor"),
    (0x814, "WIFI_CRYPTO_VALID", "valid bitmap for 32 native key slots"),
    (0xC40, "WIFI_INTERRUPT_MASK", "Wi-Fi MAC interrupt mask"),
    (0xC48, "WIFI_INTERRUPT_EVENT", "latched Wi-Fi MAC interrupt events"),
    (0xC4C, "WIFI_INTERRUPT_CLEAR", "write-one-to-clear Wi-Fi events"),
    (0xC70, "WIFI_RX_ADDRESS_HIGH", "high address bits for RX DMA descriptors"),
    (0xCB4, "WIFI_TX_QUEUE_STATE_CLEAR", "clear completed TX queue bits"),
    (0xCB8, "WIFI_TX_QUEUE_STATE", "completed TX queue bitmap"),
    (0xDDC, "WIFI_RESET_CONTROL", "Wi-Fi reset strobe and ready acknowledgement"),
]:
    modeled(wifi + offset, name, function)
for queue in range(6):
    modeled(wifi + 0xD6C - queue * 0x10, f"WIFI_TX_QUEUE{queue}_CONTROL", "queue enable and TX descriptor pointer")
    modeled(wifi + 0xD68 - queue * 0x10, f"WIFI_TX_QUEUE{queue}_TIMEOUT", "queue transmission timeout")
    modeled(wifi + 0xD60 - queue * 0x10, f"WIFI_TX_QUEUE{queue}_PROTECTION", "queue RTS/protection configuration")
    for offset, suffix, function in [
        (0x14EC, "COMPLETION", "TX completion status"),
        (0x14E8, "COMPLETION_COUNT", "TX completion count"),
        (0x14DC, "BA_STATUS", "TX block-ack status and starting sequence"),
        (0x14D8, "BA_BITMAP_LOW", "TX block-ack bitmap bits 0..31"),
        (0x14D4, "BA_BITMAP_HIGH", "TX block-ack bitmap bits 32..63"),
    ]:
        modeled(wifi + offset - queue * 0x74, f"WIFI_TX_QUEUE{queue}_{suffix}", function)
for agreement in range(8):
    for offset, suffix, function in [
        (0x290, "CONTROL", "RX block-ack agreement control"),
        (0x294, "MAC_HIGH", "RX block-ack peer address high bits"),
        (0x298, "MAC_LOW", "RX block-ack peer address low bits"),
        (0x2A0, "SEQUENCE", "RX block-ack window origin"),
        (0x2A8, "BITMAP_LOW", "RX block-ack receive bitmap bits 0..31"),
        (0x2B0, "BITMAP_HIGH", "RX block-ack receive bitmap bits 32..63"),
    ]:
        modeled(wifi + offset - agreement * 0x28, f"WIFI_RX_BA{agreement}_{suffix}", function)
for slot in range(32):
    for word in range(10):
        modeled(wifi + 0x1800 + slot * 0x28 + word * 4, f"WIFI_CRYPTO_SLOT{slot}_WORD{word}", "native Wi-Fi crypto table word")

# BLE link-layer scheduler and AES ECB/CCM accelerator registers.
ble_bb = BASES["esp32c6.ble-baseband-registers"]
for offset, name, function in [
    (0x028, "BLE_SCHEDULER_KICK", "submit the scheduler head descriptor"),
    (0x02C, "BLE_SCHEDULER_STOP", "stop the current BLE schedule"),
    (0x304, "BLE_INTERRUPT_ENABLE0", "BLE event enable bank 0"),
    (0x308, "BLE_INTERRUPT_CLEAR0", "BLE event clear bank 0"),
    (0x30C, "BLE_INTERRUPT_RAW0", "BLE raw event bank 0"),
    (0x314, "BLE_INTERRUPT_ENABLE1", "BLE event enable bank 1"),
    (0x318, "BLE_INTERRUPT_CLEAR1", "BLE event clear bank 1"),
    (0x31C, "BLE_INTERRUPT_RAW1", "BLE raw event bank 1"),
    (0x8FC, "BLE_SCHEDULER_HEAD", "first pending BLE schedule descriptor"),
    (0x900, "BLE_SCHEDULER_CURRENT", "active BLE schedule descriptor and ownership"),
    (0x904, "BLE_SCHEDULER_NEXT", "successor BLE schedule descriptor"),
    (0x924, "BLE_TIMER_CURRENT", "hardware-owned BLE scheduler time"),
    (0x960, "BLE_CURRENT_TX_BUFFER", "current BLE TX buffer descriptor"),
    (0x964, "BLE_CURRENT_RX_BUFFER", "current BLE RX buffer descriptor"),
    (0xFF0, "BLE_BASEBAND_RESET", "BLE baseband reset edge"),
]:
    modeled(ble_bb + offset, name, function)
ble_control = BASES["esp32c6.ble-control-registers"]
for offset, name, function in [
    (0x404, "BLE_ECB_START", "start AES-ECB operation"),
    (0x40C, "BLE_ECB_LENGTH", "AES-ECB transfer length"),
    (0x420, "BLE_ECB_INPUT_ADDRESS", "AES-ECB input buffer address"),
    (0x424, "BLE_ECB_OUTPUT_ADDRESS", "AES-ECB output buffer address"),
    (0x428, "BLE_CCM_START", "start AES-CCM operation"),
    (0x42C, "BLE_CCM_RESET", "reset AES-CCM state"),
    (0x430, "BLE_CCM_CONFIG", "AES-CCM direction and message length"),
    (0x434, "BLE_CCM_RESULT", "AES-CCM authentication result"),
    (0x438, "BLE_CCM_INPUT_ADDRESS", "AES-CCM input buffer address"),
    (0x43C, "BLE_CCM_OUTPUT_ADDRESS", "AES-CCM output buffer address"),
    (0x450, "BLE_CCM_COUNTER_LOW", "AES-CCM packet counter low word"),
    (0x454, "BLE_CCM_COUNTER_IV0", "AES-CCM counter high byte and IV byte 0"),
    (0x458, "BLE_CCM_IV1", "AES-CCM IV bytes 1..4"),
    (0x45C, "BLE_CCM_IV2", "AES-CCM IV bytes 5..7"),
    (0x460, "BLE_CCM_AAD", "AES-CCM associated-data header"),
    (0x4C0, "BLE_CCM_STATUS", "AES-CCM completion status"),
    (0x4C4, "BLE_ECB_STATUS", "AES-ECB completion status"),
]:
    modeled(ble_control + offset, name, function)
for word in range(4):
    modeled(ble_control + 0x410 + word * 4, f"BLE_ECB_KEY_WORD{word}", "AES-ECB 128-bit key word")
    modeled(ble_control + 0x440 + word * 4, f"BLE_CCM_KEY_WORD{word}", "AES-CCM 128-bit key word")

# PHY timekeeping and wakeup interrupt state.
phy = BASES["esp32c6.phy-registers"]
for offset, name, function in [
    (0x000, "PHY_TIME", "free-running simulation time low word"),
    (0x014, "PHY_TSF_LATCH_CONTROL", "latch TSF into low/high registers"),
    (0x020, "PHY_TSF_LOW", "latched TSF low word"),
    (0x024, "PHY_TSF_HIGH", "latched TSF high word"),
    (0x0A8, "PHY_POWER_INTERRUPT_ENABLE", "PHY timer interrupt enables"),
    (0x0AC, "PHY_POWER_INTERRUPT_RAW", "raw PHY timer interrupts"),
    (0x0B0, "PHY_POWER_INTERRUPT_STATUS", "enabled PHY timer interrupts"),
    (0x0B4, "PHY_POWER_INTERRUPT_CLEAR", "write-one-to-clear PHY timer interrupts"),
]:
    modeled(phy + offset, name, function)
for timer in range(4):
    modeled(phy + 0x074 + timer * 8, f"PHY_TSF_TIMER{timer}_CONTROL", "TSF timer enable and wakeup control")
    modeled(phy + 0x078 + timer * 8, f"PHY_TSF_TIMER{timer}_TARGET", "TSF timer target")

# BLE sleep/synchronization timers, including their explicit scheduler-state
# and monotonic-time rejection paths.
ble_modem = BASES["esp32c6.ble-modem-registers"]
for offset, name, function in [
    (0x010, "BLE_RTC_INTERRUPT_ENABLE", "arm RTC wake after a valid compare is programmed"),
    (0x014, "BLE_RTC_INTERRUPT_CLEAR", "clear RTC wake and return its state machine to idle"),
    (0x01C, "BLE_TIMER_INTERRUPT_RAW", "latched BLE synchronization-timer expiry"),
    (0x024, "BLE_RTC_TIMER0_PENDING", "latched RTC timer-0 pending state"),
    (0x034, "BLE_RTC_INTERRUPT_STATUS", "latched RTC wake interrupt status"),
    (0x044, "BLE_TIMER_CURRENT", "read-only 100 kHz BLE sleep counter"),
    (0x058, "BLE_TIMER_COMPARE", "future-only BLE synchronization compare"),
    (0x060, "BLE_RTC_COMPARE", "future-only RTC wake compare; must precede enable"),
]:
    modeled(ble_modem + offset, name, function)

# IEEE 802.15.4: every register with a nonzero writable mask plus modeled
# counters/status words. Names stay deliberately generic where semantics are
# not established beyond the mask.
ieee = BASES["esp32c6.ieee802154"]
ieee_masks = {
    0x00: 0xFF, 0x04: 0xFBC058EB, 0x08: 0xFFFF, 0x0C: 0xFFFF,
    0x10: 0xFFFFFFFF, 0x14: 0xFFFFFFFF, 0x18: 0xFFFF, 0x1C: 0xFFFF,
    0x20: 0xFFFFFFFF, 0x24: 0xFFFFFFFF, 0x28: 0xFFFF, 0x2C: 0xFFFF,
    0x30: 0xFFFFFFFF, 0x34: 0xFFFFFFFF, 0x38: 0xFFFF, 0x3C: 0xFFFF,
    0x40: 0xFFFFFFFF, 0x44: 0xFFFFFFFF, 0x48: 0x7F, 0x4C: 0x1F,
    0x50: 0x0F00FFFF, 0x54: 0xFFFF, 0x58: 0x03FF00FF, 0x5C: 0xFFFF,
    0x60: 0x1FFF, 0x64: 0x1FFF, 0x68: 0x7FFFFFFF, 0x6C: 0xFFFF0001,
    0x70: 0x1FF, 0x78: 0x7FFFFFFF, 0x7C: 0xFFFFFFFF,
    0xA8: 0xFFFFFFFF, 0xB0: 0xFFFFFFFF, 0xB8: 0xFFFFFFFF,
    0xC4: 0xFFFFFFFF, 0xC8: 0xFFFFFFFF, 0xD0: 0xFFFFFFFF,
    0xD4: 0x7, 0xE0: 0xFFFFFFFF, 0xE4: 0x03000007,
    0xF0: 0xFFFFFFFF, 0xF4: 0xFFFFFFFF, 0x128: 0x7F01,
    0x180: 0x7FFF, 0x184: 0xFFFFFFFF,
}
for offset in range(0x100, 0x124, 4):
    ieee_masks[offset] = 0xFFFFFFFF
for offset in range(0x12C, 0x144, 4):
    ieee_masks[offset] = 0xFFFFFFFF
ieee_functions = {
    0x00: ("IEEE802154_COMMAND", "execute TX, RX, CCA, energy-detect, test, stop, or timer command"),
    0x48: ("IEEE802154_CHANNEL", "802.15.4 channel selection"),
    0x4C: ("IEEE802154_TX_POWER", "802.15.4 transmit-power selection"),
    0x50: ("IEEE802154_ED_DURATION", "energy-detection duration in symbols"),
    0x60: ("IEEE802154_EVENT_ENABLE", "802.15.4 event enable mask"),
    0x64: ("IEEE802154_EVENT_CLEAR", "write-one-to-clear event state"),
    0xA8: ("IEEE802154_TIMER0_THRESHOLD", "MAC timer 0 threshold"),
    0xB0: ("IEEE802154_TIMER1_THRESHOLD", "MAC timer 1 threshold"),
    0xD0: ("IEEE802154_TX_DMA", "TX DMA descriptor address"),
    0xE0: ("IEEE802154_RX_DMA", "RX DMA descriptor address"),
    0x128: ("IEEE802154_SECURITY_CONTROL", "frame-security control"),
    0x180: ("IEEE802154_COUNTER_CLEAR", "write-one-to-clear statistic counters"),
    0x184: ("IEEE802154_DATE", "hardware date/version value"),
}
for offset, mask in ieee_masks.items():
    name, function = ieee_functions.get(offset, (f"IEEE802154_REGISTER_{offset:03X}", "modeled writable register; field semantics are not yet established"))
    modeled(ieee + offset, name, function, "high" if offset in ieee_functions else "medium", mask, 0x00220622 if offset == 0x184 else 0)
for offset, name, function in [
    (0x64, "IEEE802154_EVENT_STATE", "latched 802.15.4 events"),
    (0xAC, "IEEE802154_TIMER0_VALUE", "elapsed MAC timer 0 ticks"),
    (0xB4, "IEEE802154_TIMER1_VALUE", "elapsed MAC timer 1 ticks"),
]:
    modeled(ieee + offset, name, function)
for offset in range(0x144, 0x180, 4):
    modeled(ieee + offset, f"IEEE802154_COUNTER_{offset:03X}", "modeled MAC statistic counter", "medium", None, 0)

# MODEM_SYSCON/LPCON have per-register masks for their leading control words;
# reset-edge and clock meanings are only named where code establishes them.
for region, masks in [
    ("esp32c6.modem-syscon", [0x1, 0xFFE00000, 0xFFC00000, 0xFFFFFF00, 0xEFC7C500, 0x00FFFFFF, 0x00FFFFFF, 0xFFFFFFFF, 0xFF, 0x0FFFFFFF]),
    ("esp32c6.modem-lpcon", [0x3, 0xFFFF, 0xFFFF, 0xFFFF, 0x1, 0x3, 0xF, 0x3FF, 0xFFFF0000, 0xF, 0x000FFFFF, 0x0FFFFFFF]),
]:
    for index, mask in enumerate(masks):
        address = BASES[region] + index * 4
        name = ("MODEM_CLOCK_ENABLE" if region.endswith("syscon") and index == 5 else
                "MODEM_RESET_CONTROL" if region.endswith("syscon") and index == 4 else
                "MODEM_LP_RESET_CONTROL" if region.endswith("lpcon") and index == 9 else
                f"{region.rsplit('.', 1)[1].replace('-', '_').upper()}_{index}")
        function = ("Wi-Fi/BLE/802.15.4 clock-domain gates" if region.endswith("syscon") and index == 5 else
                    "radio-domain reset edges" if index in (4, 9) else
                    "modeled modem-control word; field semantics are not yet established")
        modeled(address, name, function, "high" if "CLOCK" in name or "RESET" in name else "medium", mask, 0)

# All 64 mapped analog-I2C words can act as packed command slots when the
# command's slave selector matches the word offset. Offsets 0x04 and 0x08 are
# unconditional command ports; 0x18 also carries the BBPLL completion bit.
analog = BASES["esp32c6.i2c-ana-mst"]
for word in range(0x100 // 4):
    offset = word * 4
    function = "packed analog-I2C command/result slot selected by the slave byte"
    if offset in (0x04, 0x08):
        function = "unconditional packed analog-I2C command/result port"
    elif offset == 0x18:
        function += "; BBPLL calibration-done status bit 24"
    modeled(analog + offset, f"ANALOG_I2C_COMMAND_{word:02X}", function, "medium")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hex32(value: int) -> str:
    return f"0x{value & 0xFFFFFFFF:08x}"


def region_for_address(address: int) -> str:
    candidates = [(base, region) for region, base in BASES.items() if base <= address]
    if not candidates:
        raise ValueError(f"no ESP32-C6 RF region for {address:#x}")
    return max(candidates)[1]


def implementation_source(region: str) -> str | None:
    return {
        "esp32c6.wifi-mac-registers": "crates/remu-devices/src/esp_c6_wifi_mac.rs",
        "esp32c6.ieee802154": "crates/remu-devices/src/esp_c6_ieee802154.rs",
        "esp32c6.ble-modem-registers": "crates/remu-devices/src/esp_c6_ble_modem.rs",
        "esp32c6.i2c-ana-mst": "crates/remu-devices/src/esp_radio_aux.rs",
        "esp32c6.power-detector": "crates/remu-devices/src/esp_c6_radio.rs",
        "esp32c6.ble-baseband-registers": "crates/remu-devices/src/esp_c6_radio.rs",
        "esp32c6.ble-control-registers": "crates/remu-devices/src/esp_c6_radio.rs",
        "esp32c6.modem-syscon": "crates/remu-devices/src/esp_c6_radio.rs",
        "esp32c6.modem-lpcon": "crates/remu-devices/src/esp_c6_radio.rs",
        "esp32c6.phy-registers": "crates/remu-devices/src/esp_c6_radio.rs",
    }.get(region)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bus", type=Path, required=True)
    parser.add_argument("--requirements", type=Path, required=True)
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()

    records = json.loads(args.bus.read_text())
    requirements = json.loads(args.requirements.read_text())
    grouped: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for record in records:
        region = record["region"]
        if region == "esp32c6.uart0":
            continue
        grouped[(region, int(record["address"]))].append(record)

    all_registers = set(grouped)
    all_registers.update((region_for_address(address), address) for address in KNOWN)
    registers = []
    for region, address in sorted(all_registers, key=lambda item: item[1]):
        accesses = grouped.get((region, address), [])
        reads = [item for item in accesses if item["kind"] == "Read"]
        writes = [item for item in accesses if item["kind"] == "Write"]
        read_values = sorted({int(item["value"]) for item in reads})
        write_values = sorted({int(item["value"]) for item in writes})
        changed_values = sorted({
            int(item["post_value"])
            for item in writes
            if item.get("pre_value") != item.get("post_value")
        })
        known = KNOWN.get(address)
        registers.append({
            "address": hex32(address),
            "offset": hex32(address - BASES[region]),
            "region": region,
            "name": known[0] if known else None,
            "technical_function": known[1] if known else "unknown; factual observed values only",
            "confidence": known[2] if known else "observed-only",
            "semantically_implemented": known is not None,
            "trace_observed": bool(accesses),
            "implementation_source": implementation_source(region) if known else None,
            "evidence_sources": [
                *(["pinned-oracle-bus-trace"] if accesses else []),
                *(["renvo-functional-model"] if known else []),
            ],
            "writable_mask": hex32(known[3]) if known and known[3] is not None else None,
            "reset_value": hex32(known[4]) if known and known[4] is not None else None,
            "access_observed": (
                "read-write" if reads and writes else "read" if reads else "write" if writes else "not-observed"
            ),
            "read_count": len(reads),
            "write_count": len(writes),
            "read_values": [hex32(value) for value in read_values],
            "write_values": [hex32(value) for value in write_values],
            "changed_post_values": [hex32(value) for value in changed_values],
            "first_at": min((int(item["at"]) for item in accesses), default=None),
            "last_at": max((int(item["at"]) for item in accesses), default=None),
            "observational_pcs": sorted({hex32(int(item["pc"])) for item in accesses}),
        })

    output = {
        "schema": "remu.c6-rf-register-reference.v1",
        "scope": "software-emulator-only",
        "claim_policy": {
            "high": "causal behavior implemented and supported by bounded trace evidence",
            "medium": "implemented behavior with incomplete field semantics",
            "observed-only": "address and values are factual; technical function is unknown",
            "implemented-only": "behavior exists in Renvo but this pinned trace did not access it",
            "pc": "observational provenance only; never used for runtime dispatch",
        },
        "provenance": {
            "bus": str(args.bus),
            "bus_sha256": digest(args.bus),
            "requirements": str(args.requirements),
            "requirements_sha256": digest(args.requirements),
            "source_ledger_entries": requirements["source_ledger_entries"],
            "implementation_sources": sorted({
                source for region in BASES if (source := implementation_source(region))
            }),
        },
        "statistics": {
            "records": sum(len(items) for items in grouped.values()),
            "registers": len(registers),
            "trace_observed_registers": sum(item["trace_observed"] for item in registers),
            "semantically_implemented_registers": sum(item["semantically_implemented"] for item in registers),
            "implemented_only_registers": sum(item["semantically_implemented"] and not item["trace_observed"] for item in registers),
            "regions": len({item["region"] for item in registers}),
            "distinct_observed_values": sum(
                len(item["read_values"]) + len(item["write_values"])
                for item in registers
            ),
            "named_registers": sum(item["name"] is not None for item in registers),
            "explicit_unknowns": sum(item["name"] is None for item in registers),
        },
        "registers": registers,
    }
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(output, indent=2) + "\n")

    by_region: dict[str, list[dict]] = defaultdict(list)
    for register in registers:
        by_region[register["region"]].append(register)
    lines = [
        "# ESP32-C6 RF register and observed-value reference",
        "",
        "This is an emulator-only, evidence-bounded reference generated from the pinned",
        "C6 public-API oracle bus trace. It is not a silicon programming manual. `high`,",
        "`medium`, and `observed-only` are claim strengths; unknown fields remain unknown.",
        "Program counters are provenance only and never affect runtime behavior.",
        "",
        f"The union contains **{len(registers)} registers**: **{output['statistics']['trace_observed_registers']} trace-observed** and **{output['statistics']['semantically_implemented_registers']} semantically implemented** (with overlap). The trace contains **{output['statistics']['distinct_observed_values']} distinct per-register read/write value observations**; **{output['statistics']['explicit_unknowns']} observed registers retain explicit semantic unknowns** across {len(by_region)} RF regions.",
        "The companion JSON contains every value and observational PC; tables below show",
        "all addresses but abbreviate value sets longer than eight entries.",
        "Modeled entries also name the repository source that implements their behavior.",
        "",
        "## Semantically implemented registers",
        "",
        "| Address | Name | Function | Mask / reset | Confidence |",
        "|---|---|---|---|---|",
    ]
    for item in registers:
        if item["name"]:
            mask_reset = f"{item['writable_mask'] or 'unknown'} / {item['reset_value'] or 'unknown'}"
            lines.append(f"| `{item['address']}` | `{item['name']}` | {item['technical_function']} | `{mask_reset}` | {item['confidence']} |")
    for region, items in sorted(by_region.items()):
        lines += ["", f"## `{region}`", "", "| Address (offset) | Access/counts | Observed values | Meaning |", "|---|---|---|---|"]
        for item in items:
            values = item["write_values"] or item["read_values"]
            shown = ", ".join(f"`{value}`" for value in values[:8])
            if len(values) > 8:
                shown += f", … ({len(values)} total; see JSON)"
            counts = f"{item['access_observed']} R{item['read_count']}/W{item['write_count']}"
            meaning = item["technical_function"]
            lines.append(f"| `{item['address']}` (`{item['offset']}`) | {counts} | {shown or '—'} | {meaning} |")
    args.markdown.parent.mkdir(parents=True, exist_ok=True)
    args.markdown.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
