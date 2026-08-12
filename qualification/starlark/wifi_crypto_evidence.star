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
