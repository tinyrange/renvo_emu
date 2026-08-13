"""Bounded genuine-firmware probe for the native Wi-Fi TX descriptor boundary.

This evidence-only agent driver stops on a firmware queue kick, then reads the
live hardware-owned DMA descriptor and payload before the Rust machine services
it. It never hooks a symbol or supplies peripheral behavior.
"""

_LAYOUTS = {
    "esp32c6": {
        "region": "esp32c6.wifi-mac-registers",
        "queue_high": 0x600a4d6c,
        "queue_low": 0x600a4d1c,
        "queue_stride": 16,
        "descriptor_base": 0x40800000,
        "protection_high": 0x600a4d60,
        "queue_mask": 0xc0000000,
        "queue_expected": 0xc0000000,
        "wire_word": True,
        "warmup_instructions": 10000000,
    },
    "esp32s3": {
        "region": "esp32s3.wifi-mac-registers",
        "queue_high": 0x60033d08,
        "queue_low": 0x60033cd0,
        "queue_stride": 8,
        "descriptor_base": 0x3fc00000,
        "protection_high": 0x60033d08,
        "queue_mask": 0xc0000000,
        "queue_expected": 0xc0000000,
        "wire_word": False,
        "warmup_instructions": 35000000,
    },
}

def _u32le(data, offset):
    return data[offset] | data[offset + 1] << 8 | data[offset + 2] << 16 | data[offset + 3] << 24

def _queue(layout, address):
    if address < layout["queue_low"] or address > layout["queue_high"]:
        return None
    distance = layout["queue_high"] - address
    if distance % layout["queue_stride"] != 0:
        return None
    return distance // layout["queue_stride"]

def main():
    layout = _LAYOUTS[machine.target()]
    machine.run(instructions = layout["warmup_instructions"])
    machine.capture_bus(
        start = layout["queue_low"],
        end = layout["queue_high"] + 4,
        regions = [layout["region"]],
        kinds = ["write"],
        capacity = 1024,
    )
    queue_count = (layout["queue_high"] - layout["queue_low"]) // layout["queue_stride"] + 1
    cursor = 0
    protection_values = {}
    for queue_index in range(queue_count):
        machine.masked_write_watchpoint(
            layout["queue_high"] - queue_index * layout["queue_stride"],
            layout["queue_mask"],
            layout["queue_expected"],
        )
    for unused in range(128):
        final_run = machine.run(instructions = 1000000)
        page = machine.bus_events(cursor = cursor, limit = 1024)
        cursor = page["next_cursor"]
        if page["missed_before_cursor"]:
            fail("native Wi-Fi queue capture lost evidence")
        for event in page["events"]:
            access = event["access"]
            for protection_queue in range(queue_count):
                protection_address = layout["protection_high"] - protection_queue * layout["queue_stride"]
                if access["address"] == protection_address:
                    protection_values[protection_address] = access["value"]
            queue = _queue(layout, access["address"])
            value = access["value"]
            if queue == None or value & layout["queue_mask"] != layout["queue_expected"] or value & 0x000fffff == 0:
                continue
            descriptor_address = layout["descriptor_base"] | value & 0x000fffff
            descriptor = machine.read(descriptor_address, 32)
            descriptor_control = _u32le(descriptor, 0)
            buffer_address = _u32le(descriptor, 4)
            if buffer_address == 0:
                continue
            payload = machine.read(buffer_address, 128)
            wire_word = _u32le(payload, 0)
            s3_ampdu_record = machine.target() == "esp32s3" and descriptor_control & 0x20000000 != 0
            mpdu_offset = 8 if s3_ampdu_record else 0
            protection_address = layout["protection_high"] - queue * layout["queue_stride"]
            machine.stop_bus_capture()
            return {
                "target": machine.target(),
                "at": access["at"],
                "queue": queue,
                "queue_address": access["address"],
                "queue_value": value,
                "protection_address": protection_address,
                "protection_value": protection_values.get(protection_address, 0),
                "descriptor_address": descriptor_address,
                "wire_word": wire_word,
                "s3_ampdu_record": s3_ampdu_record,
                "lmac_private_bits": wire_word & 0xfffff000 if layout["wire_word"] else None,
                "descriptor_words": [
                    _u32le(descriptor, 0),
                    buffer_address,
                    _u32le(descriptor, 8),
                    _u32le(descriptor, 12),
                    _u32le(descriptor, 16),
                    _u32le(descriptor, 20),
                    _u32le(descriptor, 24),
                    _u32le(descriptor, 28),
                ],
                "payload_words": [
                    _u32le(payload, 0),
                    _u32le(payload, 4),
                    _u32le(payload, 8),
                    _u32le(payload, 12),
                ],
                "mpdu_words": [
                    _u32le(payload, mpdu_offset),
                    _u32le(payload, mpdu_offset + 4),
                    _u32le(payload, mpdu_offset + 8),
                    _u32le(payload, mpdu_offset + 12),
                ],
                "run": {
                    "reason": final_run["reason"],
                    "stats": final_run["stats"],
                    "trace_digest": final_run["trace_digest"],
                },
            }
        if final_run["stats"]["instructions"] >= 80000000:
            fail("genuine firmware did not kick a native Wi-Fi queue within the execution bound")
    fail("genuine firmware exceeded the native Wi-Fi queue watchpoint budget")
