"""Deterministic external station for genuine native A-MPDU qualification.

The peer operates only on submitted RF frames. It cannot inspect the machine,
guest memory, registers, symbols, files, clocks, or networks. Genuine firmware
owns ARP, UDP, ADDBA negotiation, descriptor assembly, retries and completion.
"""

_STATION = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01]
_STATION_IP = [192, 168, 4, 2]

def _response(request, frame, bytes, delay = 16, duration = 80, phy = None):
    start = request["end"] + delay
    spectrum = frame["spectrum"]
    return {
        "start": start,
        "end": start + duration,
        "protocol": "wifi",
        "center_khz": spectrum["center_khz"],
        "bandwidth_khz": spectrum["bandwidth_khz"],
        "phy": frame["phy"] if phy == None else phy,
        "bytes": bytes,
        "power_dbm": -35,
    }

def _normal_ack_transmitter(bytes):
    if len(bytes) < 16:
        return None
    frame_type = (bytes[0] >> 2) & 3
    if frame_type != 0 and frame_type != 2:
        return None
    if bytes[4] & 1:
        return None
    subtype = (bytes[0] >> 4) & 15
    if frame_type == 2 and subtype & 8:
        has_address_four = (bytes[1] & 3) == 3
        qos_offset = 30 if has_address_four else 24
        if len(bytes) <= qos_offset or ((bytes[qos_offset] >> 5) & 3) != 0:
            return None
    return bytes[10:16]

def _addba_response(bytes):
    if len(bytes) < 33 or bytes[0:2] != [0xd0, 0x00]:
        return None
    if bytes[24:26] != [3, 0]:
        return None
    return (
        [0xd0, 0, 0, 0] +
        bytes[10:16] + _STATION + bytes[16:22] + [0, 0] +
        [3, 1, bytes[26], 0, 0] + bytes[27:31]
    )

def _arp_response(bytes):
    if len(bytes) < 60 or ((bytes[0] >> 2) & 3) != 2:
        return None
    qos = ((bytes[0] >> 4) & 8) != 0
    payload = 26 if qos else 24
    if bytes[payload:payload + 8] != [0xaa, 0xaa, 3, 0, 0, 0, 8, 6]:
        return None
    arp = payload + 8
    if bytes[arp + 6:arp + 8] != [0, 1] or bytes[arp + 24:arp + 28] != _STATION_IP:
        return None
    ap = bytes[arp + 8:arp + 14]
    ap_ip = bytes[arp + 14:arp + 18]
    return (
        [0x08, 0x01, 0, 0] + ap + _STATION + ap + [0, 0] +
        [0xaa, 0xaa, 3, 0, 0, 0, 8, 6] +
        [0, 1, 8, 0, 6, 4, 0, 2] +
        _STATION + _STATION_IP + ap + ap_ip
    )

def _compressed_block_ack(mpdus):
    if len(mpdus) < 1:
        return None
    first = mpdus[0]
    if len(first) < 26:
        return None
    sequence = ((first[22] | first[23] << 8) >> 4) & 0xfff
    tid = first[24] & 0xf
    control = 4 | tid << 12
    return (
        [0x94, 0, 0, 0] + first[10:16] + _STATION +
        [control & 0xff, control >> 8, (sequence << 4) & 0xff, sequence >> 4] +
        [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    )

def on_event(event, state):
    request = event["request"]
    frame = request["frame"]
    if frame["protocol"] != "wifi":
        return None
    if frame.get("mpdus", []):
        block_ack = _compressed_block_ack(frame["mpdus"])
        if block_ack != None:
            return [_response(request, frame, block_ack)]
        return None

    bytes = frame["bytes"]
    responses = []
    transmitter = _normal_ack_transmitter(bytes)
    if transmitter != None:
        responses.append(_response(request, frame, [0xd4, 0, 0, 0] + transmitter))
    addba = _addba_response(bytes)
    if addba != None:
        responses.append(_response(request, frame, addba, delay = 112))
    arp = _arp_response(bytes)
    if arp != None:
        responses.append(_response(request, frame, arp, delay = 112))
    return responses
