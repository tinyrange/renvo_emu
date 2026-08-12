"""Deterministic external 802.11 ACK peer for genuine-firmware gates.

The callback sees only RF submissions. It cannot access the machine, guest
memory, registers, symbols, files, clocks, or networks. Guest firmware owns
all retry state; this peer merely emits the over-the-air control response that
a receiving station would send for an ACK-required unicast frame.
"""

def normal_ack_transmitter(frame):
    if len(frame) < 16:
        return None
    frame_type = (frame[0] >> 2) & 3
    if frame_type != 0 and frame_type != 2:
        return None
    if frame[4] & 1:
        return None
    subtype = (frame[0] >> 4) & 15
    if frame_type == 2 and subtype & 8:
        has_address_four = (frame[1] & 3) == 3
        qos_offset = 30 if has_address_four else 24
        if len(frame) <= qos_offset or ((frame[qos_offset] >> 5) & 3) != 0:
            return None
    return frame[10:16]

def on_event(event, state):
    request = event["request"]
    radio_frame = request["frame"]
    if radio_frame["protocol"] != "wifi":
        return None
    receiver = normal_ack_transmitter(radio_frame["bytes"])
    if receiver == None:
        return None
    start = request["end"] + 16
    spectrum = radio_frame["spectrum"]
    return [{
        "start": start,
        "end": start + 80,
        "protocol": "wifi",
        "center_khz": spectrum["center_khz"],
        "bandwidth_khz": spectrum["bandwidth_khz"],
        "phy": radio_frame["phy"],
        "bytes": [0xd4, 0, 0, 0] + receiver,
        "power_dbm": -35,
    }]
