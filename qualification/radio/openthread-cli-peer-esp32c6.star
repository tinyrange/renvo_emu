"""Deterministic Thread child used by the genuine ESP32-C6 OpenThread gate.

The peer reacts only to frames emitted through the isolated RF medium. It
performs the standards-facing parent/child exchange in Starlark while all C6
radio commands, DMA, interrupts, AES and coexistence remain native LLE paths.
Call repl() inside on_event() and pass --radio-repl to inspect live traffic.
"""

PAN_ID = 0x1234
CHANNEL_CENTER_KHZ = 2405000
PEER_EXT = [0x02, 0, 0, 0, 0, 0, 0, 1]
PEER_WIRE = [1, 0, 0, 0, 0, 0, 0, 2]
PEER_IP = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
ALL_ROUTERS = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]
MLE_KEY = [0x8d, 0x03, 0x43, 0xd3, 0xb5, 0x2e, 0x2d, 0x80, 0xf9, 0xfd, 0x7c, 0x97, 0xd5, 0x53, 0x46, 0xcb]
MAC_KEY = [0x91, 0xd4, 0x71, 0x86, 0x00, 0x96, 0xc6, 0xfe, 0x17, 0x24, 0x9f, 0xcc, 0xe3, 0x81, 0x7d, 0xc2]
PARENT_CHALLENGE = [0, 1, 2, 3, 4, 5, 6, 7]

initial_state = {
    "phase": "discover",
    "leader_wire": [],
    "leader_ext": [],
    "leader_ip": [],
    "leader_challenge": [],
    "mle_counter": 1,
    "mac_counter": 1,
}

def le16(value):
    return [value & 0xff, (value >> 8) & 0xff]

def be16(value):
    return [(value >> 8) & 0xff, value & 0xff]

def be32(value):
    return [(value >> 24) & 0xff, (value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff]

def read_le32(value):
    return value[0] | (value[1] << 8) | (value[2] << 16) | (value[3] << 24)

def bswap32(value):
    return ((value & 0xff) << 24) | ((value & 0xff00) << 8) | ((value >> 8) & 0xff00) | ((value >> 24) & 0xff)

def link_local(ext):
    iid = list(ext)
    iid[0] = iid[0] ^ 2
    return [0xfe, 0x80, 0, 0, 0, 0, 0, 0] + iid

def internet_checksum(data):
    value = list(data)
    if len(value) % 2:
        value.append(0)
    total = 0
    for index in range(0, len(value), 2):
        total += (value[index] << 8) | value[index + 1]
    total = (total & 0xffff) + (total >> 16)
    total = (total & 0xffff) + (total >> 16)
    return (~total) & 0xffff

def mle_datagram(source_ip, destination_ip, counter, plaintext):
    security = [0x15] + [counter & 0xff, (counter >> 8) & 0xff, (counter >> 16) & 0xff, (counter >> 24) & 0xff] + [0, 0, 0, 0, 1]
    protected = ieee802154_protect(
        source_ip + destination_ip + security,
        plaintext,
        MLE_KEY,
        PEER_EXT,
        bswap32(counter),
        5,
    )
    return [0] + security + protected["payload"] + protected["mic"]

def udp_checksum(source_ip, destination_ip, datagram):
    length = 8 + len(datagram)
    udp = [0x4d, 0x4c, 0x4d, 0x4c] + be16(length) + [0, 0] + datagram
    return internet_checksum(source_ip + destination_ip + be32(length) + [0, 0, 0, 17] + udp)

def peer_frame(start, bytes):
    return {
        "start": start,
        "end": start + len(bytes) * 32,
        "protocol": "ieee802154",
        "center_khz": CHANNEL_CENTER_KHZ,
        "bandwidth_khz": 2000,
        "phy": "ieee802154-oqpsk-250k",
        "bytes": bytes,
        "power_dbm": -40,
    }

def ack_frame(sequence, start):
    return peer_frame(start, ieee802154_fcs([2, 0, sequence]))

def parent_request(start, sequence, counter):
    plaintext = [
        9,
        1, 1, 0x0f,
        3, 8,
    ] + PARENT_CHALLENGE + [
        14, 1, 0x80,
        18, 2, 0, 5,
    ]
    datagram = mle_datagram(PEER_IP, ALL_ROUTERS, counter, plaintext)
    checksum = udp_checksum(PEER_IP, ALL_ROUTERS, datagram)
    mac = [0x41, 0xd8, sequence] + le16(PAN_ID) + [0xff, 0xff] + PEER_WIRE
    lowpan = [0x7f, 0x3b, 2, 0xf0, 0x4d, 0x4c, 0x4d, 0x4c] + be16(checksum) + datagram
    return peer_frame(start, ieee802154_fcs(mac + lowpan))

def child_id_request(state, start, sequence, counter):
    plaintext = [
        11,
        4, 8,
    ] + state["leader_challenge"] + [
        5, 4, 0, 0, 0, 2,
        8, 4, 0, 0, 0, 2,
        1, 1, 0x0f,
        2, 4, 0, 0, 1, 0x2c,
        18, 2, 0, 5,
        13, 3, 10, 12, 9,
    ]
    datagram = mle_datagram(PEER_IP, state["leader_ip"], counter, plaintext)
    checksum = udp_checksum(PEER_IP, state["leader_ip"], datagram)
    mac = [0x61, 0xdc, sequence] + le16(PAN_ID) + state["leader_wire"] + PEER_WIRE
    lowpan = [0x7f, 0x33, 0xf0, 0x4d, 0x4c, 0x4d, 0x4c] + be16(checksum) + datagram
    return peer_frame(start, ieee802154_fcs(mac + lowpan))

def echo_response(state, start, sequence, counter, request_payload):
    icmp = [0x81, 0, 0, 0] + request_payload[7:]
    checksum = internet_checksum(
        PEER_IP + state["leader_ip"] + be32(len(icmp)) + [0, 0, 0, 58] + icmp,
    )
    icmp = [0x81, 0] + be16(checksum) + request_payload[7:]
    header = (
        [0x69, 0xdc, sequence]
        + le16(PAN_ID)
        + state["leader_wire"]
        + PEER_WIRE
        + [
            0x0d,
            counter & 0xff,
            (counter >> 8) & 0xff,
            (counter >> 16) & 0xff,
            (counter >> 24) & 0xff,
            1,
        ]
    )
    protected = ieee802154_protect(
        header,
        # Stateless link-local source and destination are both fully elided
        # and reconstructed from the extended MAC addresses.
        [0x7a, 0x33, 0x3a] + icmp,
        MAC_KEY,
        PEER_EXT,
        bswap32(counter),
        5,
    )
    return peer_frame(
        start,
        ieee802154_fcs(header + protected["payload"] + protected["mic"]),
    )

def mle_plaintext(frame, source_ip, source_ext):
    # Unicast MLE frames in this exchange use a 21-byte extended MAC header,
    # nine compressed IPv6/UDP bytes, one security-suite byte, and a ten-byte
    # MLE security header. The final six bytes are MIC-32 plus hardware FCS.
    security = frame[31:41]
    counter = read_le32(security[1:5])
    return ieee802154_unprotect(
        source_ip + PEER_IP + security,
        frame[41:-6],
        frame[-6:-2],
        MLE_KEY,
        source_ext,
        bswap32(counter),
        5,
    )

def mac_plaintext(frame, source_ext):
    counter = read_le32(frame[22:26])
    return ieee802154_unprotect(
        frame[:27],
        frame[27:-6],
        frame[-6:-2],
        MAC_KEY,
        source_ext,
        counter,
        5,
    )

def find_tlv(payload, wanted):
    cursor = 1
    for _ in range(32):
        if cursor >= len(payload):
            break
        kind = payload[cursor]
        length = payload[cursor + 1]
        value = payload[cursor + 2:cursor + 2 + length]
        if kind == wanted:
            return value
        cursor += 2 + length
    return []

def on_event(event, state):
    request = event["request"]
    frame = request["frame"]["bytes"]
    phase = state["phase"]

    if phase == "discover" and len(frame) >= 70 and frame[0:2] == [0x41, 0xd8]:
        leader_wire = frame[7:15]
        leader_ext = list(reversed(leader_wire))
        next_state = dict(state)
        next_state.update({
            "phase": "parent-requested",
            "leader_wire": leader_wire,
            "leader_ext": leader_ext,
            "leader_ip": link_local(leader_ext),
            "mle_counter": 2,
        })
        return {
            "state": next_state,
            # The leader returns to RX after its first advertisement and tasklet
            # processing. Keep this delay relative to the observed frame so it
            # remains deterministic across firmware builds and CPU timing.
            "frames": [parent_request(request["end"] + 1300000, 0x50, 1)],
        }

    if phase == "parent-requested" and len(frame) >= 80 and frame[0:2] == [0x61, 0xdc] and frame[5:13] == PEER_WIRE:
        plaintext = mle_plaintext(frame, state["leader_ip"], state["leader_ext"])
        if plaintext[0] != 10:
            fail("expected MLE Parent Response, got command %d" % plaintext[0])
        challenge = find_tlv(plaintext, 3)
        if len(challenge) != 8:
            fail("Parent Response omitted the eight-byte Challenge TLV")
        ack_start = request["end"] + 224
        child_start = ack_start + 5 * 32 + 20000
        next_state = dict(state)
        next_state.update({
            "phase": "child-id-requested",
            "leader_challenge": challenge,
            "mle_counter": 3,
        })
        return {
            "state": next_state,
            "frames": [
                ack_frame(frame[2], ack_start),
                child_id_request(next_state, child_start, 0x51, 2),
            ],
        }

    if phase == "child-id-requested" and len(frame) >= 60 and frame[0:2] == [0x79, 0xdc] and frame[5:13] == PEER_WIRE:
        plaintext = mac_plaintext(frame, state["leader_wire"])
        if plaintext[0] & 0xc0 != 0xc0 or plaintext[7:11] != [0x4d, 0x4c, 0x4d, 0x4c]:
            fail("authenticated Child ID Response omitted the expected mesh/MLE headers")
        ack_start = request["end"] + 224
        next_state = dict(state)
        next_state["phase"] = "attached"
        return {
            "state": next_state,
            "frames": [ack_frame(frame[2], ack_start)],
        }

    if phase == "attached" and len(frame) >= 33 and frame[0] & 0x08 != 0 and frame[5:13] == PEER_WIRE:
        # Authenticate every secured unicast before acknowledging it. This is
        # also the first stage of the protected ping exchange: the genuine
        # leader owns retransmission until its MAC ACK is received.
        plaintext = mac_plaintext(frame, state["leader_wire"])
        frames = [ack_frame(frame[2], request["end"] + 224)]
        next_state = state
        if plaintext[:4] == [0x7a, 0x33, 0x3a, 0x80]:
            next_state = dict(state)
            next_state.update({"phase": "echo-sent", "mac_counter": 4})
            frames.append(
                echo_response(
                    state,
                    request["end"] + 224 + 5 * 32 + 20000,
                    0x60,
                    3,
                    plaintext,
                ),
            )
        return {
            "state": next_state,
            "frames": frames,
        }

    return {"state": state, "frames": []}
