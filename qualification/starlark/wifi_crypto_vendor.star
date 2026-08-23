"""Agent-driven genuine-firmware Wi-Fi CCMP qualification.

This checked-in workflow owns the bounded emulator experiment.  It observes
the native table programmed by genuine firmware, runs the machine with the
normal LLE peripherals, and proves that descriptor-requested hardware CCMP
changes the protected ESP-NOW payload before it reaches the RF medium.
"""

load("//qualification/starlark:agent_automation.star", "drain_bus", "drain_radio")
load(
    "//qualification/starlark:wifi_crypto_evidence.star",
    "capture_wifi_crypto",
    "require_espnow_ccmp_programming",
)

_INSTRUCTION_BUDGETS = {
    "esp32c6": 20000000,
    "esp32s3": 30000000,
}
_PEER = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01]
_PLAINTEXT = [0x43, 0x43, 0x4d, 0x50]

def _contains(haystack, needle):
    if len(needle) == 0:
        return True
    for offset in range(len(haystack) - len(needle) + 1):
        if haystack[offset:offset + len(needle)] == needle:
            return True
    return False

def _protected_espnow(event):
    if event.get("event") != "submitted":
        return None
    request = event.get("request")
    if request == None:
        return None
    frame = request.get("frame")
    if frame == None or frame.get("protocol") != "wifi" or frame.get("origin") != "emulated":
        return None
    bytes = frame.get("bytes")
    if bytes == None or len(bytes) != 59:
        return None
    if bytes[0:2] != [0xd0, 0x40] or bytes[4:10] != _PEER:
        return None
    if bytes[27] & 0x20 == 0 or (bytes[27] >> 6) & 3 != 3:
        fail("protected ESP-NOW frame used an invalid CCMP ExtIV/key-ID header")
    if _contains(bytes[32:], _PLAINTEXT):
        fail("native hardware CCMP left the secure ESP-NOW payload in plaintext")
    if bytes[51:56] == [0x00, 0xef, 0xbe, 0xad, 0xde]:
        fail("native hardware CCMP left the firmware MIC reservation unchanged")
    return {
        "length": len(bytes),
        "receiver": bytes[4:10],
        "transmitter": bytes[10:16],
        "ccmp_header": bytes[24:32],
        "encrypted_prefix": bytes[32:40],
        "mic": bytes[-8:],
        "plaintext_absent": True,
    }

def main():
    target = machine.target()
    if target not in _INSTRUCTION_BUDGETS:
        fail("genuine Wi-Fi crypto qualification supports only esp32c6 and esp32s3")

    capture_wifi_crypto(machine, capacity = 8192)
    budget = _INSTRUCTION_BUDGETS[target]
    result = machine.run(instructions = budget)
    if result["reason"] != "InstructionLimit" or result["stats"]["instructions"] != budget:
        fail("genuine Wi-Fi firmware stopped before the qualification budget")

    bus = drain_bus(machine, maximum = 8192)
    if not bus["complete"]:
        fail("native Wi-Fi crypto capture exceeded its evidence budget")
    crypto = require_espnow_ccmp_programming(target, bus["events"])

    radio = drain_radio(machine, maximum = 4096)
    if not radio["complete"]:
        fail("radio evidence exceeded its qualification budget")
    protected = None
    for event in radio["events"]:
        candidate = _protected_espnow(event)
        if candidate != None:
            if protected != None:
                fail("genuine firmware emitted more than one matching protected ESP-NOW frame")
            protected = candidate
    if protected == None:
        fail("genuine firmware did not emit the protected ESP-NOW frame")

    return {
        "schema": "remu.radio-wifi-crypto-agent.v1",
        "target": target,
        "run": {
            "reason": result["reason"],
            "instructions": result["stats"]["instructions"],
        },
        "crypto": crypto,
        "protected_frame": protected,
        "evidence": {
            "bus_events": len(bus["events"]),
            "radio_events": len(radio["events"]),
            "bus_loss": False,
        },
    }
