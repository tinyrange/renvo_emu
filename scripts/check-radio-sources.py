#!/usr/bin/env python3
"""Validate the immutable radio source ledger and chip inventory offline."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from urllib.parse import urlparse


ROOT = Path(__file__).resolve().parent.parent
LEDGER_PATH = ROOT / "qualification/radio/source-ledger.json"
INVENTORY_PATH = ROOT / "qualification/radio/inventory.json"
ROM_REQUIREMENTS_PATH = ROOT / "qualification/radio/rom-requirements.json"
WIFI_SOFTAP_VENDOR_REQUIREMENTS_PATH = (
    ROOT / "qualification/radio/wifi-softap-vendor-requirements.json"
)
COEXISTENCE_VENDOR_REQUIREMENTS_PATH = (
    ROOT / "qualification/radio/coexistence-vendor-requirements.json"
)
CUSTOM_STACK_REQUIREMENTS_PATH = (
    ROOT / "qualification/radio/custom-stack-probe/requirements.json"
)
BLE_VENDOR_REQUIREMENTS_PATH = ROOT / "qualification/radio/ble-vendor-requirements.json"
IEEE802154_VENDOR_REQUIREMENTS_PATH = (
    ROOT / "qualification/radio/ieee802154-vendor-requirements.json"
)
OPENTHREAD_VENDOR_REQUIREMENTS_PATH = (
    ROOT / "qualification/radio/openthread-vendor-requirements.json"
)
LEGAL_STATE_CONTRACT_PATH = ROOT / "qualification/radio/legal-state-contract.json"
LEGALITY_SOURCE_PATH = ROOT / "crates/remu-radio/src/legality.rs"
RADIO_ROM_SOURCE_PATH = ROOT / "crates/remu-machines/src/radio_rom.rs"
RADIO_WIT_PATH = ROOT / "crates/remu-web/wit/renvo.wit"
SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_COMMIT = re.compile(r"^[0-9a-f]{40}$")
EXPECTED_PROTOCOLS = {
    "esp32c6": {
        "wifi": "required",
        "bluetooth-le": "required",
        "ieee802154": "required",
    },
    "esp32s3": {
        "wifi": "required",
        "bluetooth-le": "required",
        "ieee802154": "not-present",
    },
}
EXPECTED_INTERRUPTS = {"esp32c6": list(range(13)), "esp32s3": list(range(12))}
EXPECTED_LEGAL_STATE_RULES = [
    "monotonic-time",
    "domain-ready",
    "monotonic-reset-generation",
    "dma-address",
    "dma-length",
    "interrupt-domain",
    "operation-overlap",
    "completion-without-operation",
    "scheduler-state",
    "memory-mapping",
    "coexistence-ownership",
]
BLOCK_STATUSES = {"required", "discovery-required", "disputed-revision", "not-present"}
EXPECTED_ROM_REQUIREMENTS = {
    "esp32c6": {
        "architecture": "riscv32",
        "rom_file": "esp32c6_rev0_rom.elf",
        "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
        "rom_start": "0x40000000",
        "rom_end": "0x40050000",
        "minimum_instructions": 20_000_000,
        "minimum_wifi_tx_frames": 4,
        "radio_input": "qualification/radio/wifi-association-vendor-esp32c6.json",
        "expected_station_mac": [82, 69, 2, 0, 0, 0],
        "expected_auth_request_length": 30,
        "expected_association_request_length": 45,
        "calibration_regions": [
            "esp32c6.i2c-ana-mst",
            "esp32c6.power-detector",
            "esp32c6.phy-registers",
            "esp32c6.phy-i2c-command-memory",
        ],
        "required_stop": "InstructionLimit",
        "required_uart_substrings": [
            "Calling app_main()",
            "wifi driver task:",
            "wifi firmware version:",
            "phy_version",
            "wifi:mode : sta",
            "wifi:enable tsf",
            "REMU_VENDOR_WIFI_SCAN_DONE result=0",
            "REMU_VENDOR_WIFI_CONNECT_START result=0",
            "wifi:state: init -> auth",
            "wifi:state: auth -> assoc",
            "wifi:state: assoc -> run",
            "wifi:connected with REMU-AP, aid = 1",
            "REMU_VENDOR_WIFI_CONNECT_DONE connected=1 reason=-1",
        ],
    },
    "esp32s3": {
        "architecture": "xtensa",
        "rom_file": "esp32s3_rev0_rom.elf",
        "rom_sha256": "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        "rom_start": "0x40000000",
        "rom_end": "0x40070000",
        "minimum_instructions": 24_000_000,
        "minimum_wifi_tx_frames": 4,
        "radio_input": "qualification/radio/wifi-association-vendor-esp32s3.json",
        "expected_station_mac": [85, 68, 51, 34, 17, 0],
        "expected_auth_request_length": 30,
        "expected_association_request_length": 40,
        "calibration_regions": [
            "esp32s3.fe-registers",
            "esp32s3.phy-registers",
            "esp32s3.agc-registers",
            "esp32s3.nrx-registers",
            "esp32s3.bb-registers",
        ],
        "required_stop": "InstructionLimit",
        "required_uart_substrings": [
            "Project name:     remu_vendor_wifi_probe",
            "wifi driver task:",
            "wifi firmware version:",
            "phy_version",
            "wifi:mode : sta",
            "wifi:enable tsf",
            "REMU_VENDOR_WIFI_SCAN_DONE result=0",
            "REMU_VENDOR_WIFI_CONNECT_START result=0",
            "wifi:state: init -> auth",
            "wifi:state: auth -> assoc",
            "wifi:state: assoc -> run",
            "wifi:connected with REMU-AP, aid = 1",
            "REMU_VENDOR_WIFI_CONNECT_DONE connected=1 reason=-1",
        ],
    },
}
EXPECTED_WIFI_SOFTAP_VENDOR_REQUIREMENTS = {
    "esp32c6": {
        "rom_file": "esp32c6_rev0_rom.elf",
        "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
        "rom_start": "0x40000000",
        "rom_end": "0x40050000",
        "minimum_instructions": 20_000_000,
        "radio_input": "qualification/radio/wifi-softap-esp-now-vendor-esp32c6.json",
        "expected_ap_mac": [82, 69, 2, 0, 0, 1],
        "expected_beacon_length": 171,
        "expected_authentication_response_length": 30,
        "expected_association_response_length": 47,
        "required_uart_substrings": [
            "wifi:mode : softAP (52:45:02:00:00:01)",
            "REMU_VENDOR_SOFTAP_STARTED",
            "REMU_VENDOR_SOFTAP_CONFIG mac=52:45:02:00:00:01",
            "channel=1 ssid=REMU-SOFTAP",
            "REMU_VENDOR_ESPNOW_TX_START result=0",
            "wifi:station: 02:aa:bb:cc:dd:01 join, AID=1",
            "REMU_VENDOR_SOFTAP_STATION_CONNECTED mac=02:aa:bb:cc:dd:01 aid=1",
            "REMU_VENDOR_ESPNOW_RX source=02:aa:bb:cc:dd:01 destination=52:45:02:00:00:01 length=4 data=deadbeef",
            "REMU_VENDOR_SOFTAP_DONE started=1 station=1 espnow_rx=1",
        ],
    },
    "esp32s3": {
        "rom_file": "esp32s3_rev0_rom.elf",
        "rom_sha256": "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        "rom_start": "0x40000000",
        "rom_end": "0x40070000",
        "minimum_instructions": 30_000_000,
        "radio_input": "qualification/radio/wifi-softap-esp-now-vendor-esp32s3.json",
        "expected_ap_mac": [85, 68, 51, 34, 17, 1],
        "expected_beacon_length": 168,
        "expected_authentication_response_length": 30,
        "expected_association_response_length": 44,
        "required_uart_substrings": [
            "wifi:mode : softAP (55:44:33:22:11:01)",
            "REMU_VENDOR_SOFTAP_STARTED",
            "REMU_VENDOR_SOFTAP_CONFIG mac=55:44:33:22:11:01",
            "channel=1 ssid=REMU-SOFTAP",
            "REMU_VENDOR_ESPNOW_TX_START result=0",
            "wifi:station: 02:aa:bb:cc:dd:01 join, AID=1",
            "REMU_VENDOR_SOFTAP_STATION_CONNECTED mac=02:aa:bb:cc:dd:01 aid=1",
            "REMU_VENDOR_ESPNOW_RX source=02:aa:bb:cc:dd:01 destination=55:44:33:22:11:01 length=4 data=deadbeef",
            "REMU_VENDOR_SOFTAP_DONE started=1 station=1 espnow_rx=1",
        ],
    },
}
EXPECTED_COEXISTENCE_VENDOR_REQUIREMENTS = {
    "esp32c6": {
        "rom_file": "esp32c6_rev0_rom.elf",
        "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
        "rom_start": "0x40000000",
        "rom_end": "0x40050000",
        "minimum_instructions": 24_000_000,
        "radio_input": "qualification/radio/coexistence-vendor-esp32c6.json",
        "expected_host_frames": 3,
        "expected_ap_mac": [82, 69, 2, 0, 0, 1],
        "expected_beacon_length": 169,
        "expected_association_response_length": 47,
        "expected_emulated_protocols": ["wifi", "bluetooth-le", "ieee802154"],
        "expected_reset_events": 5,
        "required_uart_substrings": [
            "wifi:mode : softAP (52:45:02:00:00:01)",
            "REMU_VENDOR_COEX_WIFI_STARTED",
            "REMU_VENDOR_COEX_BLE_INIT result=0",
            "REMU_VENDOR_COEX_BLE_ADV_START result=0 wifi=1",
            "REMU_VENDOR_COEX_BLE_ADV_STOP result=0",
            "REMU_VENDOR_COEX_BLE_SCAN_START result=0",
            "REMU_VENDOR_COEX_IEEE802154_INIT result=0 channel=11",
            "REMU_VENDOR_COEX_IEEE802154_TX_START result=0 wifi=1 ble_scan=1",
            "REMU_VENDOR_COEX_IEEE802154_TX_DONE length=4 01 00 2a a5",
            "wifi:station: 02:aa:bb:cc:dd:01 join, AID=1",
            "REMU_VENDOR_COEX_BLE_SCAN_REPORT type=3 length=6 rssi=-80 02 01 06 02 09 52",
            "REMU_VENDOR_COEX_DONE wifi=1 ble_adv=1 ble_scan=1",
        ],
    },
    "esp32s3": {
        "rom_file": "esp32s3_rev0_rom.elf",
        "rom_sha256": "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
        "rom_start": "0x40000000",
        "rom_end": "0x40070000",
        "minimum_instructions": 36_000_000,
        "radio_input": "qualification/radio/coexistence-vendor-esp32s3.json",
        "expected_host_frames": 7,
        "expected_ap_mac": [85, 68, 51, 34, 17, 1],
        "expected_beacon_length": 166,
        "expected_association_response_length": 44,
        "expected_emulated_protocols": ["wifi", "bluetooth-le"],
        "expected_reset_events": 4,
        "required_uart_substrings": [
            "wifi:mode : softAP (55:44:33:22:11:01)",
            "REMU_VENDOR_COEX_WIFI_STARTED",
            "REMU_VENDOR_COEX_BLE_INIT result=0",
            "REMU_VENDOR_COEX_BLE_ADV_START result=0 wifi=1",
            "REMU_VENDOR_COEX_BLE_ADV_STOP result=0",
            "REMU_VENDOR_COEX_BLE_SCAN_START result=0",
            "wifi:station: 02:aa:bb:cc:dd:01 join, AID=1",
            "REMU_VENDOR_COEX_BLE_SCAN_REPORT type=3 length=6 rssi=-80 02 01 06 02 09 52",
            "REMU_VENDOR_COEX_DONE wifi=1 ble_adv=1 ble_scan=1",
        ],
    },
}
EXPECTED_CUSTOM_STACK_REQUIREMENTS = {
    "esp32c6": {
        "source": "esp32c6.c",
        "toolchain": "toolchains/riscv32-esp-gcc-esp32c6.toml",
        "artifact": "esp32c6-custom-radio.elf",
        "max_instructions": 100_000,
        "wifi_frame_data_offset": 8,
        "required_regions": [
            "esp32c6.wifi-mac-registers",
            "esp32c6.ble-baseband-registers",
            "esp32c6.ieee802154",
            "esp32c6.modem-syscon",
            "esp32c6.interrupt-matrix",
            "esp32c6.plic-machine",
        ],
        "families_exercised": ["wifi-mac", "ble-link-layer", "ieee802154-mac"],
    },
    "esp32s3": {
        "source": "esp32s3.c",
        "toolchain": "toolchains/xtensa-esp-gcc-esp32s3.toml",
        "artifact": "esp32s3-custom-radio.elf",
        "max_instructions": 100_000,
        "wifi_frame_data_offset": 0,
        "required_regions": [
            "esp32s3.wifi-mac-registers",
            "esp32s3.ble-exchange-memory-registers",
            "esp32s3.interrupt-matrix",
        ],
        "families_exercised": ["wifi-mac", "ble-link-layer"],
    },
}
EXPECTED_BLE_VENDOR_REQUIREMENTS = {
    "schema": "remu.radio-ble-vendor-requirements.v7",
    "source_ledger_entry": "esp-rom-elfs-20260528",
    "esp_idf_container": "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
    "firmware_project": "qualification/radio/vendor-ble-probe",
    "firmware_elf": "remu_vendor_ble_probe.elf",
    "chips": {
        "esp32c6": {
            "rom_file": "esp32c6_rev0_rom.elf",
            "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
            "rom_start": "0x40000000",
            "rom_end": "0x40050000",
            "minimum_instructions": 20_000_000,
            "radio_input": "qualification/radio/ble-advertisement-vendor-esp32c6.json",
            "required_uart_substrings": [
                "REMU_VENDOR_BLE_INIT result=0",
                "REMU_VENDOR_BLE_SEQUENCE result=1",
                "REMU_VENDOR_BLE_ADV_START result=0",
                "REMU_VENDOR_BLE_ADV_STOP result=0",
                "REMU_VENDOR_BLE_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_EXT_ADV_START result=0",
                "REMU_VENDOR_BLE_EXT_ADV_STOP result=0",
                "REMU_VENDOR_BLE_EXT_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_CONNECT status=0 handle=0",
                "REMU_VENDOR_BLE_CONNECTION_READY handle=0",
                "REMU_VENDOR_BLE_LTK_STORE result=0 handle=0",
                "REMU_VENDOR_BLE_MTU handle=0 cid=4 value=247",
                "REMU_VENDOR_BLE_PHY status=0 handle=0 tx=2 rx=2",
                "REMU_VENDOR_BLE_ENCRYPTION status=0 handle=0 encrypted=1",
                "REMU_VENDOR_BLE_ENCRYPTION_OBSERVED value=1",
                "REMU_VENDOR_BLE_DISCONNECT reason=531 handle=0",
                "REMU_VENDOR_BLE_REMOTE_TERMINATE observed=1",
                "REMU_VENDOR_BLE_CONNECTED_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_SCAN_START result=0",
                "REMU_VENDOR_BLE_EXT_SCAN_REPORT props=16 status=0 legacy=3 length=6 rssi=-80 02 01 06 02 09 52",
            ],
            "expected_native_tx": {
                "legacy": {
                    "center_khz": 2_480_000,
                    "phy": "ble-1m",
                    "bytes": [66, 21, 6, 5, 4, 3, 2, 193, 2, 1, 6, 11, 9, 82, 101, 110, 118, 111, 45, 66, 76, 69, 49],
                },
                "extended_primary": {
                    "center_khz": 2_480_000,
                    "phy": "ble-1m",
                    "bytes": [71, 7, 6, 24, 1, 48, 11, 15, 32],
                },
                "extended_auxiliary": {
                    "center_khz": 2_428_000,
                    "phy": "ble-2m",
                    "bytes": [71, 24, 9, 9, 6, 5, 4, 3, 2, 193, 1, 48, 2, 1, 6, 10, 9, 82, 101, 110, 118, 111, 45, 69, 88, 84],
                },
            },
            "expected_connection_tx": [
                {"center_khz": 2_424_000, "phy": "ble-1m", "bytes": [7, 6, 12, 14, 229, 2, 0, 0]},
                {"center_khz": 2_436_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_446_000, "phy": "ble-1m", "bytes": [7, 9, 14, 255, 127, 1, 7, 144, 27, 0, 0]},
                {"center_khz": 2_456_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_466_000, "phy": "ble-1m", "bytes": [7, 9, 20, 251, 0, 144, 66, 251, 0, 144, 66]},
                {"center_khz": 2_476_000, "phy": "ble-1m", "bytes": [10, 7, 3, 0, 4, 0, 3, 0, 1]},
                {"center_khz": 2_410_000, "phy": "ble-1m", "bytes": [5, 0]},
                {"center_khz": 2_420_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_432_000, "phy": "ble-1m", "bytes": [7, 3, 23, 7, 7]},
                {"center_khz": 2_428_000, "phy": "ble-2m", "bytes": [9, 0]},
                {"center_khz": 2_438_000, "phy": "ble-2m", "bytes": [5, 0]},
                {"center_khz": 2_448_000, "phy": "ble-2m", "bytes": [11, 13, 4, 169, 236, 143, 50, 213, 120, 27, 190, 6, 99, 192, 35]},
                {"center_khz": 2_458_000, "phy": "ble-2m", "bytes": [7, 1, 5]},
                {"center_khz": 2_458_000, "phy": "ble-2m", "bytes": [11, 5, 183, 131, 33, 0, 81]},
                {"center_khz": 2_468_000, "phy": "ble-2m", "bytes": [5, 4, 127, 67, 237, 21]},
                {"center_khz": 2_478_000, "phy": "ble-2m", "bytes": [9, 4, 37, 145, 144, 80]},
            ],
        },
        "esp32s3": {
            "rom_file": "esp32s3_rev0_rom.elf",
            "rom_sha256": "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
            "rom_start": "0x40000000",
            "rom_end": "0x40070000",
            "minimum_instructions": 24_000_000,
            "radio_input": "qualification/radio/ble-advertisement-vendor-esp32s3.json",
            "required_uart_substrings": [
                "REMU_VENDOR_BLE_INIT result=0",
                "REMU_VENDOR_BLE_SEQUENCE result=1",
                "REMU_VENDOR_BLE_ADV_START result=0",
                "REMU_VENDOR_BLE_ADV_STOP result=0",
                "REMU_VENDOR_BLE_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_EXT_ADV_START result=0",
                "REMU_VENDOR_BLE_EXT_ADV_STOP result=0",
                "REMU_VENDOR_BLE_EXT_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_CONNECT status=0 handle=1",
                "REMU_VENDOR_BLE_CONNECTION_READY handle=1",
                "REMU_VENDOR_BLE_LTK_STORE result=0 handle=1",
                "REMU_VENDOR_BLE_MTU handle=1 cid=4 value=247",
                "REMU_VENDOR_BLE_PHY status=0 handle=1 tx=2 rx=2",
                "REMU_VENDOR_BLE_ENCRYPTION status=0 handle=1 encrypted=1",
                "REMU_VENDOR_BLE_ENCRYPTION_OBSERVED value=1",
                "REMU_VENDOR_BLE_DISCONNECT reason=531 handle=1",
                "REMU_VENDOR_BLE_REMOTE_TERMINATE observed=1",
                "REMU_VENDOR_BLE_CONNECTED_ADV_REMOVE result=0",
                "REMU_VENDOR_BLE_SCAN_START result=0",
                "REMU_VENDOR_BLE_EXT_SCAN_REPORT props=16 status=0 legacy=3 length=6 rssi=-80 02 01 06 02 09 52",
            ],
            "expected_native_tx": {
                "legacy": {
                    "center_khz": 2_480_000,
                    "phy": "ble-1m",
                    "bytes": [98, 21, 2, 17, 34, 51, 68, 85, 2, 1, 6, 11, 9, 82, 101, 110, 118, 111, 45, 66, 76, 69, 49],
                },
                "extended_primary": {
                    "center_khz": 2_480_000,
                    "phy": "ble-1m",
                    "bytes": [39, 7, 6, 24, 10, 39, 32, 5, 32],
                },
                "extended_auxiliary": {
                    "center_khz": 2_470_000,
                    "phy": "ble-2m",
                    "bytes": [103, 24, 9, 9, 6, 5, 4, 3, 2, 193, 10, 0, 2, 1, 6, 10, 9, 82, 101, 110, 118, 111, 45, 69, 88, 84],
                },
            },
            "expected_connection_tx": [
                {"center_khz": 2_414_000, "phy": "ble-1m", "bytes": [23, 6, 12, 9, 229, 2, 22, 0]},
                {"center_khz": 2_424_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_436_000, "phy": "ble-1m", "bytes": [23, 9, 14, 255, 249, 1, 0, 0, 0, 0, 0]},
                {"center_khz": 2_446_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_456_000, "phy": "ble-1m", "bytes": [23, 9, 20, 251, 0, 144, 66, 251, 0, 144, 66]},
                {"center_khz": 2_466_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_476_000, "phy": "ble-1m", "bytes": [22, 7, 3, 0, 4, 0, 3, 0, 1]},
                {"center_khz": 2_410_000, "phy": "ble-1m", "bytes": [9, 0]},
                {"center_khz": 2_420_000, "phy": "ble-1m", "bytes": [23, 3, 23, 7, 7]},
                {"center_khz": 2_428_000, "phy": "ble-2m", "bytes": [9, 0]},
                {"center_khz": 2_438_000, "phy": "ble-2m", "bytes": [6, 7, 3, 0, 4, 0, 3, 0, 1]},
                {"center_khz": 2_448_000, "phy": "ble-2m", "bytes": [23, 13, 4, 117, 204, 150, 89, 66, 217, 135, 254, 147, 165, 63, 51]},
                {"center_khz": 2_458_000, "phy": "ble-2m", "bytes": [9, 0]},
                {"center_khz": 2_468_000, "phy": "ble-2m", "bytes": [23, 1, 5]},
                {"center_khz": 2_478_000, "phy": "ble-2m", "bytes": [13, 4, 158, 245, 5, 206]},
                {"center_khz": 2_412_000, "phy": "ble-2m", "bytes": [19, 9, 64, 145, 48, 102, 250, 205, 206, 172, 115]},
            ],
        },
    },
}
EXPECTED_IEEE802154_VENDOR_REQUIREMENTS = {
    "schema": "remu.radio-ieee802154-vendor-requirements.v1",
    "source_ledger_entry": "esp-rom-elfs-20260528",
    "esp_idf_container": "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
    "firmware_project": "qualification/radio/vendor-ieee802154-probe",
    "firmware_elf": "remu_vendor_ieee802154_probe.elf",
    "chip": "esp32c6",
    "rom_file": "esp32c6_rev0_rom.elf",
    "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
    "rom_start": "0x40000000",
    "rom_end": "0x40050000",
    "minimum_instructions": 12_000_000,
    "radio_input": "qualification/radio/ieee802154-frame-vendor-esp32c6.json",
    "required_uart_substrings": [
        "REMU_VENDOR_IEEE802154_INIT result=0",
        "REMU_VENDOR_IEEE802154_CONFIG result=0 channel=11 promiscuous=1",
        "REMU_VENDOR_IEEE802154_TX_START result=0",
        "REMU_VENDOR_IEEE802154_TX_DONE length=4 01 00 2a a5",
        "REMU_VENDOR_IEEE802154_CCA_TX_START result=0",
        "REMU_VENDOR_IEEE802154_CCA_BUSY_DONE complete=0 failed=1 error=1",
        "REMU_VENDOR_IEEE802154_CCA_RETRY_START result=0",
        "REMU_VENDOR_IEEE802154_CCA_RETRY_DONE complete=1 failed=0 error=-1",
        "REMU_VENDOR_IEEE802154_RX_START result=0",
        "REMU_VENDOR_IEEE802154_RX_DONE length=6 01 00 02 aa rssi=-80 lqi=63",
        "REMU_VENDOR_IEEE802154_ED_START result=0",
        "REMU_VENDOR_IEEE802154_ED_DONE power=-128",
        "REMU_VENDOR_IEEE802154_MULTIPAN result=0 mask=3 pan0=1234 short0=5678 pan1=abcd short1=1357",
        "REMU_VENDOR_IEEE802154_FILTER_RX_START result=0",
        "REMU_VENDOR_IEEE802154_FILTER_RX_REARM result=0",
        "REMU_VENDOR_IEEE802154_FILTER_RX_DONE length=11 01 08 32 cd ab 57 13 de ad mpf=1 rssi=-80 lqi=63",
        "REMU_VENDOR_IEEE802154_AUTO_ACK_RX_START result=0",
        "REMU_VENDOR_IEEE802154_AUTO_ACK_RX_DONE length=11 21 08 46 cd ab 57 13 11 22 pending=1 rssi=-80 lqi=63 callbacks=3",
        "REMU_VENDOR_IEEE802154_ACK_TX_START result=0",
        "REMU_VENDOR_IEEE802154_ACK_RX_DONE length=5 02 00 44 rssi=-80 lqi=63",
        "REMU_VENDOR_IEEE802154_NO_ACK_TX_START result=0",
        "REMU_VENDOR_IEEE802154_NO_ACK_DONE complete=0 failed=1 error=3",
    ],
}
EXPECTED_OPENTHREAD_VENDOR_REQUIREMENTS = {
    "schema": "remu.radio-openthread-vendor-requirements.v1",
    "source_ledger_entries": [
        "esp-rom-elfs-20260528",
        "esp-idf-openthread-radio-port",
    ],
    "esp_idf_container": "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
    "firmware_project": "qualification/radio/vendor-openthread-probe",
    "firmware_elf": "remu_vendor_openthread_probe.elf",
    "chip": "esp32c6",
    "rom_file": "esp32c6_rev0_rom.elf",
    "rom_sha256": "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551",
    "rom_start": "0x40000000",
    "rom_end": "0x40050000",
    "minimum_instructions": 15_000_000,
    "radio_input": "qualification/radio/openthread-frame-vendor-esp32c6.json",
    "required_uart_substrings": [
        "REMU_VENDOR_OPENTHREAD_PLATFORM result=0",
        "REMU_VENDOR_OPENTHREAD_START result=0",
        "REMU_VENDOR_OPENTHREAD_RAW_CONFIG error=0 enabled=1 promiscuous=1 caps=255",
        "REMU_VENDOR_OPENTHREAD_TX_FRAME channel=11 length=5",
        "REMU_VENDOR_OPENTHREAD_TX_START error=0",
        "REMU_VENDOR_OPENTHREAD_TX_DONE complete=1 error=0",
        "REMU_VENDOR_OPENTHREAD_RX_START error=0",
        "REMU_VENDOR_OPENTHREAD_RX_DONE complete=1 error=0 length=7 01 00 79 52 58 rssi=-80 lqi=63",
        "REMU_VENDOR_OPENTHREAD_ED_START error=0",
        "REMU_VENDOR_OPENTHREAD_ED_DONE complete=1 rssi=-128",
        "REMU_VENDOR_OPENTHREAD_SLEEP_WAKE sleep=0 wake=0",
    ],
}


class Validation:
    """Accumulate all validation failures for one useful CI report."""

    def __init__(self) -> None:
        self.errors: list[str] = []

    def require(self, condition: bool, message: str) -> None:
        if not condition:
            self.errors.append(message)


def load_json(path: Path) -> object:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def parse_hex(value: str, label: str, validation: Validation) -> int | None:
    try:
        return int(value, 16)
    except (TypeError, ValueError):
        validation.errors.append(f"{label} must be a hexadecimal string")
        return None


def validate_ledger(ledger: dict[str, object], validation: Validation) -> None:
    validation.require(
        ledger.get("schema") == "remu.radio-source-ledger.v1",
        "source ledger schema is not remu.radio-source-ledger.v1",
    )
    policy = ledger.get("policy", {})
    allowed = set(policy.get("allowed_code_licenses", []))
    validation.require(bool(allowed), "source ledger has no allowed code licenses")
    denied = {item.lower() for item in policy.get("denied_license_families", [])}
    validation.require(
        {"agpl", "gpl", "lgpl", "sspl", "unknown"}.issubset(denied),
        "source ledger must explicitly deny reciprocal and unknown license families",
    )

    sources = ledger.get("sources", [])
    validation.require(isinstance(sources, list) and bool(sources), "source ledger is empty")
    seen_ids: set[str] = set()
    for index, source in enumerate(sources):
        label = f"source[{index}]"
        source_id = source.get("id")
        validation.require(isinstance(source_id, str) and bool(source_id), f"{label} has no id")
        validation.require(source_id not in seen_ids, f"duplicate source id {source_id!r}")
        if isinstance(source_id, str):
            seen_ids.add(source_id)
        url = source.get("url", "")
        parsed = urlparse(url)
        validation.require(
            parsed.scheme == "https" and bool(parsed.netloc), f"{label} URL must use HTTPS"
        )
        validation.require(bool(source.get("revision")), f"{label} has no immutable revision")
        validation.require(
            bool(SHA256.fullmatch(source.get("content_sha256", ""))),
            f"{label} has an invalid SHA-256 digest",
        )
        if source.get("kind") in {"code", "documentation"}:
            license_name = source.get("license")
            validation.require(
                license_name in allowed,
                f"{label} license {license_name!r} is not on the allowlist",
            )
            revision = source.get("revision", "")
            validation.require(
                bool(GIT_COMMIT.fullmatch(revision)),
                f"{label} code/documentation revision must be a full Git commit",
            )
            validation.require(bool(source.get("repository")), f"{label} has no repository URL")

    validation.require(
        {
            "esp-rom-elfs-20260528",
            "esp-idf-openthread-radio-port",
        }.issubset(seen_ids),
        "OpenThread qualification sources are missing from the source ledger",
    )


def validate_inventory(inventory: dict[str, object], validation: Validation) -> None:
    validation.require(
        inventory.get("schema") == "remu.radio-inventory.v1",
        "radio inventory schema is not remu.radio-inventory.v1",
    )
    validation.require(
        inventory.get("source_ledger") == "qualification/radio/source-ledger.json",
        "radio inventory does not name the checked source ledger",
    )
    chips = inventory.get("chips", {})
    validation.require(set(chips) == set(EXPECTED_PROTOCOLS), "inventory chip set must be C6 and S3")
    for chip, expected_protocols in EXPECTED_PROTOCOLS.items():
        entry = chips.get(chip, {})
        validation.require(
            entry.get("protocols") == expected_protocols,
            f"{chip} protocol applicability matrix changed or is incomplete",
        )
        interrupts = entry.get("interrupts", [])
        sources = [item.get("source") for item in interrupts]
        validation.require(
            sources == EXPECTED_INTERRUPTS[chip],
            f"{chip} radio interrupt sources must be contiguous and complete",
        )
        names = [item.get("name") for item in interrupts]
        validation.require(
            all(isinstance(name, str) and name for name in names) and len(names) == len(set(names)),
            f"{chip} interrupt names must be non-empty and unique",
        )

        ranges: list[tuple[int, int, str]] = []
        blocks = entry.get("blocks", [])
        validation.require(bool(blocks), f"{chip} has no radio block inventory")
        for index, block in enumerate(blocks):
            label = f"{chip}.blocks[{index}]"
            validation.require(block.get("status") in BLOCK_STATUSES, f"{label} has invalid status")
            validation.require(bool(block.get("name")), f"{label} has no name")
            has_base = "base" in block
            has_size = "size" in block
            validation.require(has_base == has_size, f"{label} must specify base and size together")
            if has_base and has_size:
                base = parse_hex(block["base"], f"{label}.base", validation)
                size = parse_hex(block["size"], f"{label}.size", validation)
                if base is not None and size is not None:
                    validation.require(size > 0, f"{label}.size must be positive")
                    ranges.append((base, base + size, block["name"]))
        for index, first in enumerate(ranges):
            for second in ranges[index + 1 :]:
                validation.require(
                    first[1] <= second[0] or second[1] <= first[0],
                    f"{chip} radio blocks {first[2]} and {second[2]} overlap",
                )


def validate_rom_requirements(requirements: dict[str, object], validation: Validation) -> None:
    validation.require(
        requirements.get("schema") == "remu.radio-rom-requirements.v3",
        "radio ROM requirements schema is not remu.radio-rom-requirements.v3",
    )
    validation.require(
        requirements.get("source_ledger_entry") == "esp-rom-elfs-20260528",
        "radio ROM requirements must use the pinned source-ledger entry",
    )
    validation.require(
        requirements.get("rom_release") == "20260528",
        "radio ROM release changed from the required 20260528 artifact",
    )
    validation.require(
        requirements.get("rom_archive_sha256")
        == "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46",
        "radio ROM archive digest changed from the vendor-published checksum",
    )
    validation.require(
        requirements.get("esp_idf_container")
        == "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
        "real-ROM firmware must use the pinned ESP-IDF v6.0.2 container digest",
    )
    validation.require(
        requirements.get("firmware_project") == "qualification/radio/vendor-wifi-probe",
        "real-ROM qualification must build the repository-owned vendor Wi-Fi probe",
    )
    validation.require(
        requirements.get("firmware_elf") == "remu_vendor_wifi_probe.elf",
        "real-ROM qualification must execute the vendor Wi-Fi probe ELF",
    )
    validation.require(
        requirements.get("deterministic_replay_required") is True,
        "real-ROM qualification must require deterministic replay",
    )
    validation.require(
        requirements.get("calibration_evidence_required") is True,
        "real-ROM qualification must require firmware-observed calibration evidence",
    )
    validation.require(
        requirements.get("symbol_dispatch_allowed") is False,
        "real-ROM qualification must forbid symbol dispatch",
    )
    chips = requirements.get("chips", {})
    validation.require(
        set(chips) == set(EXPECTED_ROM_REQUIREMENTS),
        "real-ROM qualification must require both ESP32-C6 and ESP32-S3",
    )
    for chip, expected in EXPECTED_ROM_REQUIREMENTS.items():
        validation.require(
            chips.get(chip) == expected,
            f"{chip} real-ROM acceptance contract changed or is incomplete",
        )
        radio_input = ROOT / expected["radio_input"]
        validation.require(radio_input.is_file(), f"{chip} radio input is missing")
        if radio_input.is_file():
            fixture = load_json(radio_input)
            frames = fixture.get("frames", [])
            validation.require(
                fixture.get("schema") == "remu.radio-input.v1"
                and len(frames) == 4,
                f"{chip} vendor gate must use four deterministic association frames",
            )
            validation.require(
                [frame.get("bytes", [None])[0] for frame in frames]
                == [0x80, 0x80, 0xB0, 0x10],
                f"{chip} vendor gate must provide beacon, directed beacon, authentication, and association responses",
            )


def validate_wifi_softap_vendor_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements.get("schema")
        == "remu.radio-wifi-softap-vendor-requirements.v1",
        "genuine SoftAP/ESP-NOW requirements schema changed",
    )
    validation.require(
        requirements.get("source_ledger_entry") == "esp-rom-elfs-20260528"
        and requirements.get("rom_release") == "20260528"
        and requirements.get("rom_archive_sha256")
        == "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46",
        "genuine SoftAP/ESP-NOW qualification must use the pinned ROM release",
    )
    validation.require(
        requirements.get("esp_idf_container")
        == "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
        "genuine SoftAP/ESP-NOW qualification must use the pinned ESP-IDF image",
    )
    validation.require(
        requirements.get("firmware_project")
        == "qualification/radio/vendor-wifi-ap-probe"
        and requirements.get("firmware_elf") == "remu_vendor_wifi_ap_probe.elf",
        "genuine SoftAP/ESP-NOW qualification must use the repository-owned probe",
    )
    validation.require(
        requirements.get("deterministic_replay_required") is True
        and requirements.get("symbol_dispatch_allowed") is False,
        "genuine SoftAP/ESP-NOW qualification must replay exactly without symbol dispatch",
    )
    chips = requirements.get("chips", {})
    validation.require(
        chips == EXPECTED_WIFI_SOFTAP_VENDOR_REQUIREMENTS,
        "genuine SoftAP/ESP-NOW acceptance contract changed or is incomplete",
    )
    validation.require(
        (ROOT / str(requirements.get("firmware_project"))).is_dir(),
        "genuine SoftAP/ESP-NOW probe project is missing",
    )
    for chip, contract in EXPECTED_WIFI_SOFTAP_VENDOR_REQUIREMENTS.items():
        radio_input = ROOT / contract["radio_input"]
        validation.require(radio_input.is_file(), f"{chip} SoftAP radio input is missing")
        if radio_input.is_file():
            fixture = load_json(radio_input)
            frames = fixture.get("frames", [])
            validation.require(
                fixture.get("schema") == "remu.radio-input.v1"
                and [frame.get("bytes", [None])[0] for frame in frames]
                == [0xB0, 0x00, 0xD0],
                f"{chip} SoftAP gate must inject authentication, association, and ESP-NOW frames",
            )


def validate_coexistence_vendor_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements.get("schema") == "remu.radio-coexistence-vendor-requirements.v1",
        "genuine coexistence requirements schema changed",
    )
    validation.require(
        requirements.get("source_ledger_entry") == "esp-rom-elfs-20260528"
        and requirements.get("rom_release") == "20260528"
        and requirements.get("rom_archive_sha256")
        == "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46",
        "genuine coexistence qualification must use the pinned ROM release",
    )
    validation.require(
        requirements.get("esp_idf_container")
        == "espressif/idf@sha256:0d8c9773d48a327233f9c1d7c654ff0bcf133ae24503ea2e97a57cfe02b8cb67",
        "genuine coexistence qualification must use the pinned ESP-IDF image",
    )
    validation.require(
        requirements.get("firmware_project") == "qualification/radio/vendor-coex-probe"
        and requirements.get("firmware_elf") == "remu_vendor_coex_probe.elf"
        and requirements.get("sdkconfig_defaults")
        == "qualification/radio/sdkconfig.coex.defaults",
        "genuine coexistence qualification must use the repository-owned combined probe",
    )
    validation.require(
        requirements.get("deterministic_replay_required") is True
        and requirements.get("symbol_dispatch_allowed") is False,
        "genuine coexistence qualification must replay exactly without symbol dispatch",
    )
    chips = requirements.get("chips", {})
    validation.require(
        chips == EXPECTED_COEXISTENCE_VENDOR_REQUIREMENTS,
        "genuine coexistence acceptance contract changed or is incomplete",
    )
    validation.require(
        (ROOT / str(requirements.get("firmware_project"))).is_dir(),
        "genuine coexistence probe project is missing",
    )
    validation.require(
        (ROOT / str(requirements.get("sdkconfig_defaults"))).is_file(),
        "genuine coexistence sdkconfig defaults are missing",
    )
    for chip, contract in EXPECTED_COEXISTENCE_VENDOR_REQUIREMENTS.items():
        radio_input = ROOT / contract["radio_input"]
        validation.require(radio_input.is_file(), f"{chip} coexistence radio input is missing")
        if radio_input.is_file():
            fixture = load_json(radio_input)
            frames = fixture.get("frames", [])
            validation.require(
                fixture.get("schema") == "remu.radio-input.v1"
                and len(frames) == contract["expected_host_frames"]
                and [frame.get("protocol") for frame in frames[:2]] == ["wifi", "wifi"]
                and all(frame.get("protocol") == "bluetooth-le" for frame in frames[2:]),
                f"{chip} coexistence gate must inject the pinned Wi-Fi and BLE traffic",
            )


def validate_custom_stack_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements.get("schema") == "remu.radio-custom-stack-requirements.v1",
        "custom-stack requirements schema is not remu.radio-custom-stack-requirements.v1",
    )
    chips = requirements.get("chips", {})
    validation.require(
        set(chips) == set(EXPECTED_CUSTOM_STACK_REQUIREMENTS),
        "custom-stack qualification must require both ESP32-C6 and ESP32-S3",
    )
    for chip, expected in EXPECTED_CUSTOM_STACK_REQUIREMENTS.items():
        validation.require(
            chips.get(chip) == expected,
            f"{chip} custom-stack acceptance contract changed or is incomplete",
        )
        source = ROOT / "qualification/radio/custom-stack-probe" / expected["source"]
        validation.require(source.is_file(), f"{chip} custom-stack source is missing")


def validate_ble_vendor_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements == EXPECTED_BLE_VENDOR_REQUIREMENTS,
        "genuine BLE acceptance contract changed or is incomplete",
    )
    validation.require(
        (ROOT / str(requirements.get("firmware_project"))).is_dir(),
        "genuine BLE probe project is missing",
    )
    for chip, contract in EXPECTED_BLE_VENDOR_REQUIREMENTS["chips"].items():
        validation.require(
            (ROOT / contract["radio_input"]).is_file(),
            f"{chip} genuine BLE radio input is missing",
        )


def validate_ieee802154_vendor_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements == EXPECTED_IEEE802154_VENDOR_REQUIREMENTS,
        "genuine IEEE 802.15.4 acceptance contract changed or is incomplete",
    )
    validation.require(
        (ROOT / str(requirements.get("firmware_project"))).is_dir(),
        "genuine IEEE 802.15.4 probe project is missing",
    )
    validation.require(
        (ROOT / str(requirements.get("radio_input"))).is_file(),
        "genuine IEEE 802.15.4 radio input is missing",
    )


def validate_openthread_vendor_requirements(
    requirements: dict[str, object], validation: Validation
) -> None:
    validation.require(
        requirements == EXPECTED_OPENTHREAD_VENDOR_REQUIREMENTS,
        "genuine OpenThread acceptance contract changed or is incomplete",
    )
    validation.require(
        (ROOT / str(requirements.get("firmware_project"))).is_dir(),
        "genuine OpenThread probe project is missing",
    )
    validation.require(
        (ROOT / str(requirements.get("radio_input"))).is_file(),
        "genuine OpenThread radio input is missing",
    )


def validate_legal_state_contract(
    contract: dict[str, object], validation: Validation
) -> None:
    validation.require(
        contract.get("schema") == "remu.radio-legal-state-contract.v1",
        "radio legal-state contract schema is not remu.radio-legal-state-contract.v1",
    )
    policy = contract.get("policy", {})
    validation.require(
        policy.get("baseline") == "firmware-observed-only"
        and policy.get("violation") == "hard-error"
        and policy.get("diagnostic_prefix") == "illegal radio state"
        and policy.get("learning_at_runtime") is False
        and policy.get("ordinary_rf_outcomes_are_violations") is False,
        "radio legal-state policy must remain firmware-derived and fail hard",
    )
    validation.require(
        contract.get("rule_codes") == EXPECTED_LEGAL_STATE_RULES,
        "radio legal-state rule inventory changed or is incomplete",
    )
    evidence = contract.get("evidence", [])
    validation.require(
        isinstance(evidence, list) and bool(evidence),
        "radio legal-state contract has no firmware evidence",
    )
    for path in evidence:
        validation.require(
            isinstance(path, str) and (ROOT / path).is_file(),
            f"radio legal-state evidence is missing: {path!r}",
        )
    chips = contract.get("chips", {})
    expected_subsystems = {
        "esp32c6": ["wifi", "bluetooth-le", "ieee802154", "coexistence"],
        "esp32s3": ["wifi", "bluetooth-le", "coexistence"],
    }
    for chip, subsystems in expected_subsystems.items():
        chip_contract = chips.get(chip, {})
        rom = EXPECTED_ROM_REQUIREMENTS[chip]
        validation.require(
            chip_contract.get("required_rom") == rom["rom_file"]
            and chip_contract.get("required_rom_sha256") == rom["rom_sha256"],
            f"{chip} legal-state contract is not pinned to the required genuine ROM",
        )
        validation.require(
            chip_contract.get("subsystems") == subsystems,
            f"{chip} legal-state subsystem inventory changed or is incomplete",
        )
        invariants = chip_contract.get("observed_invariants", {})
        validation.require(
            isinstance(invariants, dict)
            and {"clock_reset", "wifi_dma", "bluetooth_le", "coexistence"}.issubset(
                invariants
            )
            and (chip != "esp32c6" or "ieee802154" in invariants),
            f"{chip} legal-state firmware invariants are incomplete",
        )
    legality_source = LEGALITY_SOURCE_PATH.read_text(encoding="utf-8")
    for code in EXPECTED_LEGAL_STATE_RULES:
        validation.require(
            f'"{code}"' in legality_source,
            f"radio legality implementation is missing stable rule code {code!r}",
        )
    validation.require(
        '"illegal radio state [' in legality_source,
        "radio legality implementation is missing the stable hard-error prefix",
    )

    rom_source = RADIO_ROM_SOURCE_PATH.read_text(encoding="utf-8")
    for chip, requirements in EXPECTED_ROM_REQUIREMENTS.items():
        validation.require(
            requirements["rom_sha256"] in rom_source,
            f"{chip} pinned ROM digest is not enforced by the host boundary",
        )
    validation.require(
        "verify_esp_radio_rom(target, &bytes)" in (
            ROOT / "crates/remu-cli/src/run_command.rs"
        ).read_text(encoding="utf-8"),
        "CLI radio/direct execution does not verify the pinned ROM bytes",
    )
    validation.require(
        "verify_esp_radio_rom(target, boot_rom)" in (
            ROOT / "crates/remu-web/src/lib.rs"
        ).read_text(encoding="utf-8"),
        "browser/WASI radio execution does not verify the pinned ROM bytes",
    )
    validation.require(
        "boot-rom: list<u8>" in RADIO_WIT_PATH.read_text(encoding="utf-8"),
        "browser/WASI radio API does not require caller-provided ROM bytes",
    )


def main() -> int:
    validation = Validation()
    ledger = load_json(LEDGER_PATH)
    inventory = load_json(INVENTORY_PATH)
    rom_requirements = load_json(ROM_REQUIREMENTS_PATH)
    wifi_softap_vendor_requirements = load_json(WIFI_SOFTAP_VENDOR_REQUIREMENTS_PATH)
    coexistence_vendor_requirements = load_json(COEXISTENCE_VENDOR_REQUIREMENTS_PATH)
    custom_stack_requirements = load_json(CUSTOM_STACK_REQUIREMENTS_PATH)
    ble_vendor_requirements = load_json(BLE_VENDOR_REQUIREMENTS_PATH)
    ieee802154_vendor_requirements = load_json(IEEE802154_VENDOR_REQUIREMENTS_PATH)
    openthread_vendor_requirements = load_json(OPENTHREAD_VENDOR_REQUIREMENTS_PATH)
    legal_state_contract = load_json(LEGAL_STATE_CONTRACT_PATH)
    validate_ledger(ledger, validation)
    validate_inventory(inventory, validation)
    validate_rom_requirements(rom_requirements, validation)
    validate_wifi_softap_vendor_requirements(wifi_softap_vendor_requirements, validation)
    validate_coexistence_vendor_requirements(coexistence_vendor_requirements, validation)
    validate_custom_stack_requirements(custom_stack_requirements, validation)
    validate_ble_vendor_requirements(ble_vendor_requirements, validation)
    validate_ieee802154_vendor_requirements(ieee802154_vendor_requirements, validation)
    validate_openthread_vendor_requirements(openthread_vendor_requirements, validation)
    validate_legal_state_contract(legal_state_contract, validation)
    if validation.errors:
        for error in validation.errors:
            print(f"radio audit: {error}", file=sys.stderr)
        return 1
    print(
        f"Radio audit passed: {len(ledger['sources'])} pinned sources, "
        f"{len(inventory['chips'])} chip inventories"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
