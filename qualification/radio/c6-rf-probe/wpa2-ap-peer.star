"""Deterministic open and WPA2 APs for the project-owned C6 station."""

OPEN_AP = [2, 0x52, 0x45, 0x4d, 0x55, 1]
WPA2_AP = [2, 0x52, 0x45, 0x4d, 0x55, 2]
OPEN_SSID = [82, 69, 77, 85, 45, 67, 54, 45, 79, 80, 69, 78]
WPA2_SSID = [82, 69, 77, 85, 45, 67, 54, 45, 87, 80, 65, 50]
TK = [0x3e,0x29,0x42,0x5f,0xcd,0x3c,0x44,0xd0,0x1b,0x29,0x87,0x87,0x47,0x51,0xb9,0x98]
M1 = [
    2,3,0,95,2,0,0x8a,0,16,0,0,0,0,0,0,0,1,
    0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
    16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
] + [0 for _ in range(50)]
M2 = [
    2,3,0,95,2,1,0x0a,0,16,0,0,0,0,0,0,0,1,
    0xa0,0xa1,0xa2,0xa3,0xa4,0xa5,0xa6,0xa7,
    0xa8,0xa9,0xaa,0xab,0xac,0xad,0xae,0xaf,
    0xb0,0xb1,0xb2,0xb3,0xb4,0xb5,0xb6,0xb7,
    0xb8,0xb9,0xba,0xbb,0xbc,0xbd,0xbe,0xbf,
] + [0 for _ in range(32)] + [
    0x54,0xc0,0x53,0x89,0xcf,0x1a,0xc4,0xdd,
    0xee,0x11,0x33,0x13,0xd3,0xea,0x4b,0xc9,0,0,
]
M3 = [
    2,3,0,95,2,3,0xca,0,16,0,0,0,0,0,0,0,2,
    0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
    16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
] + [0 for _ in range(32)] + [
    0x66,0x82,0x89,0x3a,0x11,0x7f,0xeb,0xfb,
    0x32,0x3f,0x12,0xd6,0xb4,0xd2,0x54,0x86,0,0,
]
M4 = [
    2,3,0,95,2,3,0x0a,0,16,0,0,0,0,0,0,0,2,
] + [0 for _ in range(64)] + [
    0x08,0xf9,0x19,0x31,0x16,0x85,0x23,0xb1,
    0x72,0x90,0x6e,0x3f,0x61,0x11,0xf8,0x38,0,0,
]
LLC_EAPOL = [0xaa,0xaa,3,0,0,0,0x88,0x8e]

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

def _header(frame, subtype, ap):
    return [subtype, 0, 0, 0] + frame[10:16] + ap + ap + [0, 0]

def _data_down(frame, ap, protected = False):
    flags = 0x42 if protected else 2
    return [8, flags, 0, 0] + frame[10:16] + ap + ap + [0, 0]

def _ack(frame):
    if len(frame) < 16 or frame[4] & 1:
        return None
    return [0xd4, 0, 0, 0] + frame[10:16]

def _probe_response(frame, ssid, ap):
    if len(frame) < 29 + len(ssid) or frame[0:2] != [0x40, 0]:
        return None
    if frame[24] != 0 or frame[25] != len(ssid) or frame[26:26 + len(ssid)] != ssid:
        return None
    return _header(frame, 0x50, ap) + [0,0,0,0,0,0,0,0,100,0,1,0] + [0,len(ssid)] + ssid + [1,1,0x82,3,1,6]

def _authentication_response(frame, ap):
    if len(frame) < 30 or frame[0:2] != [0xb0, 0] or frame[4:10] != ap:
        return None
    if frame[24:30] != [0,0,1,0,0,0]:
        return None
    return _header(frame, 0xb0, ap) + [0,0,2,0,0,0]

def _association_response(frame, ap):
    if len(frame) < 30 or frame[0:2] != [0, 0] or frame[4:10] != ap:
        return None
    return _header(frame, 0x10, ap) + [0x31,0x04,0,0,1,0] + [1,1,0x82]

def _open_l2_response(frame):
    ping = [0xaa,0xaa,3,0,0,0,0x88,0xb5,80,73,78,71]
    if len(frame) < 36 or frame[0:2] != [8,1] or frame[4:10] != OPEN_AP or frame[24:36] != ping:
        return None
    pong = [0xaa,0xaa,3,0,0,0,0x88,0xb5,80,79,78,71]
    return _data_down(frame, OPEN_AP) + pong

def _wpa2_response(frame):
    if len(frame) >= 131 and frame[4:10] == WPA2_AP and frame[24:32] == LLC_EAPOL:
        if frame[32:131] == M2:
            return _data_down(frame, WPA2_AP) + LLC_EAPOL + M3
        if frame[32:131] == M4:
            return []
        return None
    if len(frame) >= 52 and frame[4:10] == WPA2_AP and frame[0:2] == [8,0x41]:
        plain = wifi_ccmp_unprotect(frame, TK)
        ping = [0xaa,0xaa,3,0,0,0,0x88,0xb5,80,73,78,71]
        if plain[32:44] != ping:
            return None
        pong = [0xaa,0xaa,3,0,0,0,0x88,0xb5,80,79,78,71]
        response = _data_down(frame, WPA2_AP, protected = True) + [2,0,0,0x20,0,0,0,0] + pong + [0 for _ in range(8)]
        return wifi_ccmp_protect(response, TK)
    return None

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
    response = _probe_response(frame, OPEN_SSID, OPEN_AP)
    if response == None:
        response = _probe_response(frame, WPA2_SSID, WPA2_AP)
    if response == None:
        response = _authentication_response(frame, OPEN_AP)
    if response == None:
        response = _authentication_response(frame, WPA2_AP)
    if response == None:
        response = _association_response(frame, OPEN_AP)
    if response == None:
        response = _open_l2_response(frame)
    if response == None:
        response = _association_response(frame, WPA2_AP)
        if response != None:
            replies.append(_tx(request, response, delay = 512))
            replies.append(_tx(request, _data_down(frame, WPA2_AP) + LLC_EAPOL + M1, delay = 2000))
            return replies
    if response == None:
        response = _wpa2_response(frame)
    if response != None and response != []:
        replies.append(_tx(request, response, delay = 512))
    return replies if replies else None
