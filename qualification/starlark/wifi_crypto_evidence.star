"""Checked, bounded evidence helpers for native C6/S3 Wi-Fi key tables.

These helpers only configure and interpret bus capture. They do not install
keys, intercept firmware, or implement any radio behavior in Starlark.
"""

_WIFI_CRYPTO_LAYOUTS = {
    "esp32c6": {
        "valid": 0x600a4814,
        "table": 0x600a5800,
        "table_end": 0x600a5d00,
        "region": "esp32c6.wifi-mac-registers",
    },
    "esp32s3": {
        "valid": 0x60033814,
        "table": 0x60034400,
        "table_end": 0x60034900,
        "region": "esp32s3.wifi-mac-registers",
    },
}

_ESPNOW_CCMP_SLOT = 24
_ESPNOW_CCMP_MATCH = 0xccbbaa02
_ESPNOW_CCMP_CONTROL = 0xc16c01dd
_ESPNOW_CCMP_KEY_WORDS = [
    0xfefa53b8,
    0xed440253,
    0x86a9d3bd,
    0xcfed0b48,
]

def wifi_crypto_layout(target):
    """Returns the pinned native key-table capture layout for one target."""
    if target not in _WIFI_CRYPTO_LAYOUTS:
        fail("Wi-Fi crypto evidence is supported only for esp32c6 and esp32s3")
    return _WIFI_CRYPTO_LAYOUTS[target]

def capture_wifi_crypto(machine, capacity = 8192):
    """Starts a bounded write-only capture spanning one native key table."""
    layout = wifi_crypto_layout(machine.target())
    machine.capture_bus(
        start = layout["valid"],
        end = layout["table_end"],
        regions = [layout["region"]],
        kinds = ["write"],
        capacity = capacity,
    )
    return layout

def summarize_wifi_crypto_events(target, events):
    """Summarizes captured key-table writes and rejects impossible HAL classes."""
    layout = wifi_crypto_layout(target)
    valid_writes = []
    control_writes = []
    table_writes = 0
    key_writes = 0
    for event in events:
        access = event["access"]
        if access["kind"] != "Write":
            continue
        address = access["address"]
        if address == layout["valid"]:
            valid_writes.append({
                "at": access["at"],
                "mask": access["value"],
            })
            continue
        if address < layout["table"] or address >= layout["table_end"]:
            continue
        offset = address - layout["table"]
        slot = offset // 40
        field = offset % 40
        table_writes += 1
        if field >= 8:
            key_writes += 1
        if field == 4 and access["width"] == "Word" and access["value"] != 0:
            control_class = (access["value"] >> 21) & 7
            if control_class not in [3, 6, 7]:
                fail("native Wi-Fi crypto slot %s used impossible HAL control class %s" % (slot, control_class))
            control_writes.append({
                "at": access["at"],
                "slot": slot,
                "control": access["value"],
                "class": control_class,
            })
    return {
        "target": target,
        "valid_writes": valid_writes,
        "control_writes": control_writes,
        "table_writes": table_writes,
        "key_writes": key_writes,
    }

def require_wifi_crypto_programming(target, events):
    """Requires a captured valid-bit, control, and key-payload programming path."""
    summary = summarize_wifi_crypto_events(target, events)
    if not summary["valid_writes"]:
        fail("firmware did not write the native Wi-Fi crypto valid bitmap")
    if not summary["control_writes"]:
        fail("firmware did not program a nonzero native Wi-Fi crypto control word")
    if not summary["key_writes"]:
        fail("firmware did not program the native Wi-Fi crypto key payload")
    return summary

def require_espnow_ccmp_programming(target, events):
    """Requires the exact pairwise CCMP tuple emitted by the pinned firmware.

    The tuple was recovered from the permissively licensed vendor HAL and then
    observed at native MMIO while both genuine C6 and S3 firmware configured
    the encrypted qualification peer.  This assertion never supplies a key to
    the guest and does not participate in emulation.
    """
    summary = require_wifi_crypto_programming(target, events)
    layout = wifi_crypto_layout(target)
    slot_base = layout["table"] + _ESPNOW_CCMP_SLOT * 40
    observed = {}
    valid = False
    for event in events:
        access = event["access"]
        if access["kind"] != "Write" or access["width"] != "Word":
            continue
        address = access["address"]
        if address == layout["valid"] and access["value"] & (1 << _ESPNOW_CCMP_SLOT):
            valid = True
        if address >= slot_base and address < slot_base + 40:
            observed[address - slot_base] = access["value"]

    if not valid:
        fail("firmware did not mark native Wi-Fi crypto slot 24 valid")
    if observed.get(0) != _ESPNOW_CCMP_MATCH:
        fail("firmware did not program the observed encrypted ESP-NOW peer match")
    if observed.get(4) != _ESPNOW_CCMP_CONTROL:
        fail("firmware did not program the observed CCMP/interface/key-ID control tuple")
    for index in range(len(_ESPNOW_CCMP_KEY_WORDS)):
        if observed.get(8 + index * 4) != _ESPNOW_CCMP_KEY_WORDS[index]:
            fail("firmware encrypted ESP-NOW key payload diverged at word %s" % index)

    return {
        "target": target,
        "slot": _ESPNOW_CCMP_SLOT,
        "peer": [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01],
        "interface": 1,
        "cipher": "ccmp",
        "key_id": 3,
        "control_class": 3,
        "valid": True,
        "key_payload_words_verified": len(_ESPNOW_CCMP_KEY_WORDS),
        "table_writes": summary["table_writes"],
    }
