"""RF-only open-system AP for the project-owned C6 station probe."""

AP = [2, 0x52, 0x45, 0x4d, 0x55, 1]
SSID = [82, 69, 77, 85, 45, 67, 54, 45, 79, 80, 69, 78]

def _tx(request, bytes, delay = 16):
    start = request["end"] + delay
    spectrum = request["frame"]["spectrum"]
    return {
        "start": start,
        "end": start + len(bytes) * 32,
        "protocol": "wifi",
        "center_khz": spectrum["center_khz"],
        "bandwidth_khz": spectrum["bandwidth_khz"],
        "phy": "wifi-ht20",
        "bytes": bytes,
        "power_dbm": -35,
    }

def _header(request, subtype):
    station = request[10:16]
    return [subtype, 0, 0, 0] + station + AP + AP + [0, 0]

def _ack(request):
    if len(request) < 16 or request[4] & 1:
        return None
    return [0xd4, 0, 0, 0] + request[10:16]

def _probe_response(frame):
    if len(frame) < 38 or frame[0:2] != [0x40, 0]:
        return None
    if frame[24] != 0 or frame[25] != len(SSID) or frame[26:26 + len(SSID)] != SSID:
        return None
    return (
        _header(frame, 0x50)
        + [0, 0, 0, 0, 0, 0, 0, 0, 100, 0, 1, 0]
        + [0, len(SSID)] + SSID + [1, 1, 0x82, 3, 1, 6]
    )

def _authentication_response(frame):
    if len(frame) < 30 or frame[0:2] != [0xb0, 0] or frame[4:10] != AP:
        return None
    if frame[24:30] != [0, 0, 1, 0, 0, 0]:
        return None
    return _header(frame, 0xb0) + [0, 0, 2, 0, 0, 0]

def _association_response(frame):
    if len(frame) < 30 or frame[0:2] != [0, 0] or frame[4:10] != AP:
        return None
    return _header(frame, 0x10) + [0x21, 0x04, 0, 0, 1, 0] + [1, 1, 0x82]

def _l2_response(frame):
    ping = [0xaa, 0xaa, 3, 0, 0, 0, 0x88, 0xb5, 80, 73, 78, 71]
    if len(frame) < 36 or frame[0:2] != [0x08, 0x01] or frame[24:36] != ping:
        return None
    station = frame[10:16]
    pong = [0xaa, 0xaa, 3, 0, 0, 0, 0x88, 0xb5, 80, 79, 78, 71]
    return [0x08, 0x02, 0, 0] + station + AP + AP + [0, 0] + pong

def on_event(event, state):
    request = event["request"]
    radio = request["frame"]
    if radio["protocol"] != "wifi" or radio.get("origin") != "emulated":
        return None
    frame = radio["bytes"]
    replies = []
    ack = _ack(frame)
    if ack != None:
        replies.append(_tx(request, ack))
    response = _probe_response(frame)
    if response == None:
        response = _authentication_response(frame)
    if response == None:
        response = _association_response(frame)
    if response == None:
        response = _l2_response(frame)
    if response != None:
        replies.append(_tx(request, response, delay = 512))
    return replies if replies else None
