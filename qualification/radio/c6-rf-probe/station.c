/* SPDX-License-Identifier: Apache-2.0 */
#include "station.h"
#include "hal/c6/mac.h"

static const uint8_t station_address[6] = {2, 0, 0, 0, 0, 0xc6};
static const uint8_t ap_address[6] = {2, 0x52, 0x45, 0x4d, 0x55, 1};
static const uint8_t ssid[] = {'R','E','M','U','-','C','6','-','O','P','E','N'};
static uint16_t sequence_number;

enum station_state {
    STATION_IDLE,
    STATION_SCANNING,
    STATION_AUTHENTICATING,
    STATION_ASSOCIATING,
    STATION_ASSOCIATED_STATE,
};

static enum station_state state;

static void copy(uint8_t *destination, const uint8_t *source, uint32_t length)
{
    for (uint32_t index = 0; index < length; ++index) destination[index] = source[index];
}

static int same(const uint8_t *left, const uint8_t *right, uint32_t length)
{
    for (uint32_t index = 0; index < length; ++index) {
        if (left[index] != right[index]) return 0;
    }
    return 1;
}

static void management_header(uint8_t *frame, uint8_t subtype,
                              const uint8_t *destination)
{
    frame[0] = subtype;
    frame[1] = 0;
    frame[2] = 0;
    frame[3] = 0;
    copy(frame + 4, destination, 6);
    copy(frame + 10, station_address, 6);
    copy(frame + 16, ap_address, 6);
    frame[22] = (uint8_t)(sequence_number << 4);
    frame[23] = (uint8_t)(sequence_number >> 4);
    sequence_number = (sequence_number + 1u) & 0x0fffu;
}

static int send_probe_request(void)
{
    uint8_t frame[48];
    static const uint8_t broadcast[6] = {0xff,0xff,0xff,0xff,0xff,0xff};
    management_header(frame, 0x40, broadcast);
    frame[24] = 0;
    frame[25] = sizeof(ssid);
    copy(frame + 26, ssid, sizeof(ssid));
    frame[26 + sizeof(ssid)] = 1;
    frame[27 + sizeof(ssid)] = 1;
    frame[28 + sizeof(ssid)] = 0x82;
    return c6_mac_tx_frame(frame, 29u + sizeof(ssid));
}

static int send_authentication(void)
{
    uint8_t frame[30];
    management_header(frame, 0xb0, ap_address);
    frame[24] = 0;
    frame[25] = 0;
    frame[26] = 1;
    frame[27] = 0;
    frame[28] = 0;
    frame[29] = 0;
    return c6_mac_tx_frame(frame, sizeof(frame));
}

static int send_association(void)
{
    uint8_t frame[48];
    management_header(frame, 0x00, ap_address);
    frame[24] = 0x21;
    frame[25] = 0x04;
    frame[26] = 10;
    frame[27] = 0;
    frame[28] = 0;
    frame[29] = sizeof(ssid);
    copy(frame + 30, ssid, sizeof(ssid));
    uint32_t offset = 30u + sizeof(ssid);
    frame[offset++] = 1;
    frame[offset++] = 1;
    frame[offset++] = 0x82;
    return c6_mac_tx_frame(frame, offset);
}

static int send_l2_ping(void)
{
    uint8_t frame[36];
    frame[0] = 0x08;
    frame[1] = 0x01;
    frame[2] = 0;
    frame[3] = 0;
    copy(frame + 4, ap_address, 6);
    copy(frame + 10, station_address, 6);
    copy(frame + 16, ap_address, 6);
    frame[22] = (uint8_t)(sequence_number << 4);
    frame[23] = (uint8_t)(sequence_number >> 4);
    sequence_number = (sequence_number + 1u) & 0x0fffu;
    static const uint8_t payload[12] = {
        0xaa,0xaa,0x03,0x00,0x00,0x00,0x88,0xb5,'P','I','N','G'
    };
    copy(frame + 24, payload, sizeof(payload));
    return c6_mac_tx_frame(frame, sizeof(frame));
}

int c6_station_start(void)
{
    state = STATION_SCANNING;
    return send_probe_request();
}

uint32_t c6_station_receive(const uint8_t *frame, uint32_t length)
{
    if (length < 24u || !same(frame + 10, ap_address, 6)) return C6_STATION_NONE;
    if (state == STATION_SCANNING && (frame[0] & 0xfcu) == 0x50u &&
        same(frame + 4, station_address, 6)) {
        state = STATION_AUTHENTICATING;
        if (send_authentication() != 0) return C6_STATION_FAILED;
        return C6_STATION_SCANNED;
    }
    if (state == STATION_AUTHENTICATING && (frame[0] & 0xfcu) == 0xb0u &&
        length >= 30u && frame[26] == 2u && frame[28] == 0u && frame[29] == 0u) {
        state = STATION_ASSOCIATING;
        if (send_association() != 0) return C6_STATION_FAILED;
        return C6_STATION_AUTHENTICATED;
    }
    if (state == STATION_ASSOCIATING && (frame[0] & 0xfcu) == 0x10u &&
        length >= 30u && frame[26] == 0u && frame[27] == 0u) {
        state = STATION_ASSOCIATED_STATE;
        if (send_l2_ping() != 0) return C6_STATION_FAILED;
        return C6_STATION_ASSOCIATED | C6_STATION_L2_TX;
    }
    if (state == STATION_ASSOCIATED_STATE && ((frame[0] >> 2) & 3u) == 2u &&
        same(frame + 4, station_address, 6) && length >= 36u) {
        static const uint8_t pong[12] = {
            0xaa,0xaa,0x03,0x00,0x00,0x00,0x88,0xb5,'P','O','N','G'
        };
        if (same(frame + 24, pong, sizeof(pong))) return C6_STATION_L2_RX;
    }
    return C6_STATION_NONE;
}
