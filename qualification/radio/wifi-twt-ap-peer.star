"""Deterministic standards-side HE access point for the C6 TWT gate.

This RF-only peer answers open authentication and association requests from
genuine station firmware.  It advertises the same HE capabilities and
operation fields emitted by the pinned genuine C6 SoftAP image, with the
standards-defined TWT-responder capability enabled.  It cannot inspect the
machine or replace any native MAC, PHY, DMA, timer, interrupt, or power state.
"""

AP = [2, 82, 69, 77, 85, 1]

HE_ASSOCIATION_IES = [
    1, 8, 139, 150, 130, 132, 12, 24, 48, 96,
    50, 4, 108, 18, 36, 72,
    45, 26, 110, 17, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    61, 22, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    # Element 255, extension 35: first HE MAC-capability byte 0x04 enables
    # TWT responder.  All remaining bytes match genuine C6 SoftAP output.
    255, 22, 35, 4, 16, 128, 8, 0, 0, 0, 16, 0,
    27, 0, 0, 0, 0, 0, 128, 0, 252, 255, 252, 255,
    255, 7, 36, 244, 63, 0, 49, 252, 255,
    255, 14, 38, 0, 0, 164, 255, 32, 164, 255, 64, 67, 255, 96, 50, 255,
]

def _normal_ack_transmitter(frame):
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

def _transmission(request, frame, start, phy = "wifi-ht20"):
    spectrum = request["frame"]["spectrum"]
    return {
        "start": start,
        "end": start + len(frame) * 32,
        "protocol": "wifi",
        "center_khz": spectrum["center_khz"],
        "bandwidth_khz": spectrum["bandwidth_khz"],
        "phy": phy,
        "bytes": frame,
        "power_dbm": -35,
    }

def _management_header(frame, subtype):
    station = frame[10:16]
    return [subtype, 0, 0, 0] + station + AP + AP + [0, 0]

def _authentication_response(frame):
    return _management_header(frame, 0xb0) + [0, 0, 2, 0, 0, 0]

def _association_response(frame):
    return (
        _management_header(frame, 0x10)
        + [33, 4, 0, 0, 1, 192]
        + HE_ASSOCIATION_IES
    )

def _itwt_accept_response(frame):
    # S1G TWT Setup action body: category, action, dialog token, followed by
    # an individual TWT element.  Preserve the station's negotiated timing
    # fields and turn its Request/Suggest request type into Response/Accept.
    # wifi_twt_setup_cmds_t assigns ACCEPT=4; the request bit is bit 0 and
    # setup-command occupies bits 1..3 of the little-endian request type.
    if len(frame) < 44:
        return None
    if frame[24] != 22 or frame[25] != 6:
        return None
    if frame[27] != 216 or frame[28] != 15:
        return None
    element = list(frame[29:44])
    request_type = element[1] | (element[2] << 8)
    if request_type & 1 == 0:
        return None
    request_type = (request_type & ~0x0f) | (4 << 1)
    element[1] = request_type & 0xff
    element[2] = (request_type >> 8) & 0xff
    return (
        _management_header(frame, 0xd0)
        + [22, 6, frame[26], 216, 15]
        + element
    )

def on_event(event, state):
    request = event["request"]
    radio_frame = request["frame"]
    if radio_frame["protocol"] != "wifi":
        return None
    frame = radio_frame["bytes"]
    replies = []
    receiver = _normal_ack_transmitter(frame)
    if receiver != None:
        ack_start = request["end"] + 16
        replies.append(_transmission(
            request,
            [0xd4, 0, 0, 0] + receiver,
            ack_start,
            phy = radio_frame["phy"],
        ))
    if len(frame) >= 24 and frame[4:10] == AP:
        # The ACK occupies the immediate control-response window.  Keep the
        # management response outside that airtime so both frames are
        # independently receivable by the native MAC.
        response_start = request["end"] + 512
        if frame[0] & 0xfc == 0xb0:
            replies.append(_transmission(request, _authentication_response(frame), response_start))
        elif frame[0] & 0xfc == 0x00:
            replies.append(_transmission(request, _association_response(frame), response_start))
        elif frame[0] & 0xfc == 0xd0:
            response = _itwt_accept_response(frame)
            if response != None:
                replies.append(_transmission(request, response, response_start))
    return replies if replies else None
