/* SPDX-License-Identifier: Apache-2.0 */
#include "wpa2.h"

#include "crypto.h"
#include "hal/c6/mac.h"

static const uint8_t station_address[6] = {2, 0, 0, 0, 0, 0xc6};
static const uint8_t ap_address[6] = {2, 0x52, 0x45, 0x4d, 0x55, 2};
static const uint8_t ssid[] = {'R','E','M','U','-','C','6','-','W','P','A','2'};
/* PBKDF2-HMAC-SHA1("renvo-c6-wpa2", "REMU-C6-WPA2", 4096). */
static const uint8_t pmk[32] = {
    0x67,0x11,0xae,0xd4,0x0e,0x76,0x92,0x15,
    0x4a,0xcf,0xdf,0xa8,0xb0,0xfd,0x6d,0x67,
    0xbd,0x2e,0xb8,0x38,0x4c,0x98,0xdc,0x92,
    0x41,0x59,0xc3,0xee,0xa5,0x81,0x01,0xd9,
};
static const uint8_t snonce[32] = {
    0xa0,0xa1,0xa2,0xa3,0xa4,0xa5,0xa6,0xa7,
    0xa8,0xa9,0xaa,0xab,0xac,0xad,0xae,0xaf,
    0xb0,0xb1,0xb2,0xb3,0xb4,0xb5,0xb6,0xb7,
    0xb8,0xb9,0xba,0xbb,0xbc,0xbd,0xbe,0xbf,
};

enum wpa2_state {
    WPA2_IDLE,
    WPA2_SCANNING,
    WPA2_AUTHENTICATING,
    WPA2_ASSOCIATING,
    WPA2_WAIT_M1,
    WPA2_WAIT_M3,
    WPA2_WAIT_CCMP,
};

static enum wpa2_state state;
static uint16_t sequence_number;
static uint8_t ptk[64];

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

static uint32_t data_header(uint8_t *frame, int protected)
{
    frame[0] = 0x08;
    frame[1] = protected ? 0x41 : 0x01;
    frame[2] = 0;
    frame[3] = 0;
    copy(frame + 4, ap_address, 6);
    copy(frame + 10, station_address, 6);
    copy(frame + 16, ap_address, 6);
    frame[22] = (uint8_t)(sequence_number << 4);
    frame[23] = (uint8_t)(sequence_number >> 4);
    sequence_number = (sequence_number + 1u) & 0x0fffu;
    return 24;
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
    static const uint8_t body[6] = {0,0,1,0,0,0};
    copy(frame + 24, body, sizeof(body));
    return c6_mac_tx_frame(frame, sizeof(frame));
}

static int send_association(void)
{
    uint8_t frame[80];
    management_header(frame, 0x00, ap_address);
    frame[24] = 0x31;
    frame[25] = 0x04;
    frame[26] = 10;
    frame[27] = 0;
    frame[28] = 0;
    frame[29] = sizeof(ssid);
    copy(frame + 30, ssid, sizeof(ssid));
    uint32_t offset = 30u + sizeof(ssid);
    static const uint8_t rates[] = {1,1,0x82};
    static const uint8_t rsn[] = {
        48,20,1,0, 0,0x0f,0xac,4, 1,0, 0,0x0f,0xac,4,
        1,0, 0,0x0f,0xac,2, 0,0,
    };
    copy(frame + offset, rates, sizeof(rates));
    offset += sizeof(rates);
    copy(frame + offset, rsn, sizeof(rsn));
    offset += sizeof(rsn);
    return c6_mac_tx_frame(frame, offset);
}

static void make_eapol(uint8_t eapol[99], uint16_t key_info,
                       uint64_t replay, const uint8_t nonce[32])
{
    for (uint32_t index = 0; index < 99u; ++index) eapol[index] = 0;
    eapol[0] = 2;
    eapol[1] = 3;
    eapol[3] = 95;
    eapol[4] = 2;
    eapol[5] = (uint8_t)(key_info >> 8);
    eapol[6] = (uint8_t)key_info;
    eapol[8] = 16;
    for (uint32_t index = 0; index < 8u; ++index) {
        eapol[16u - index] = (uint8_t)replay;
        replay >>= 8;
    }
    copy(eapol + 17, nonce, 32);
}

static int send_eapol(uint16_t key_info, uint64_t replay, const uint8_t nonce[32])
{
    uint8_t frame[131];
    uint32_t offset = data_header(frame, 0);
    static const uint8_t llc[] = {0xaa,0xaa,3,0,0,0,0x88,0x8e};
    copy(frame + offset, llc, sizeof(llc));
    offset += sizeof(llc);
    make_eapol(frame + offset, key_info, replay, nonce);
    uint8_t digest[20];
    c6_hmac_sha1(ptk, 16, frame + offset, 99, digest);
    copy(frame + offset + 81, digest, 16);
    return c6_mac_tx_frame(frame, sizeof(frame));
}

static int verify_m3(const uint8_t *eapol)
{
    uint8_t message[99];
    uint8_t received_mic[16];
    copy(message, eapol, sizeof(message));
    copy(received_mic, message + 81, sizeof(received_mic));
    for (uint32_t index = 0; index < 16u; ++index) message[81u + index] = 0;
    uint8_t digest[20];
    c6_hmac_sha1(ptk, 16, message, sizeof(message), digest);
    return same(received_mic, digest, sizeof(received_mic));
}

static int send_ccmp_ping(void)
{
    uint8_t frame[52];
    uint32_t offset = data_header(frame, 1);
    static const uint8_t ccmp[] = {1,0,0,0x20,0,0,0,0};
    static const uint8_t ping[] = {
        0xaa,0xaa,3,0,0,0,0x88,0xb5,'P','I','N','G'
    };
    copy(frame + offset, ccmp, sizeof(ccmp));
    offset += sizeof(ccmp);
    copy(frame + offset, ping, sizeof(ping));
    offset += sizeof(ping);
    for (uint32_t index = 0; index < 8u; ++index) frame[offset++] = 0;
    return c6_mac_tx_frame(frame, offset);
}

int c6_wpa2_start(void)
{
    c6_mac_set_interface_address(station_address);
    state = WPA2_SCANNING;
    return send_probe_request();
}

uint32_t c6_wpa2_receive(const uint8_t *frame, uint32_t length)
{
    if (length < 24u || !same(frame + 10, ap_address, 6)) return C6_WPA2_NONE;
    if (state == WPA2_SCANNING && (frame[0] & 0xfcu) == 0x50u &&
        same(frame + 4, station_address, 6)) {
        state = WPA2_AUTHENTICATING;
        if (send_authentication() != 0) return C6_WPA2_FAILED;
        return C6_WPA2_SCANNED;
    }
    if (state == WPA2_AUTHENTICATING && (frame[0] & 0xfcu) == 0xb0u &&
        length >= 30u && frame[26] == 2u && frame[28] == 0u && frame[29] == 0u) {
        state = WPA2_ASSOCIATING;
        if (send_association() != 0) return C6_WPA2_FAILED;
        return C6_WPA2_AUTHENTICATED;
    }
    if (state == WPA2_ASSOCIATING && (frame[0] & 0xfcu) == 0x10u &&
        length >= 30u && frame[26] == 0u && frame[27] == 0u) {
        state = WPA2_WAIT_M1;
        return C6_WPA2_ASSOCIATED;
    }
    static const uint8_t eapol_llc[] = {0xaa,0xaa,3,0,0,0,0x88,0x8e};
    if (state == WPA2_WAIT_M1 && length >= 131u &&
        same(frame + 24, eapol_llc, sizeof(eapol_llc))) {
        const uint8_t *eapol = frame + 32;
        if (eapol[0] != 2 || eapol[1] != 3 || eapol[3] != 95 ||
            eapol[5] != 0 || eapol[6] != 0x8a || eapol[16] != 1) {
            return C6_WPA2_FAILED;
        }
        c6_wpa_prf(pmk, station_address, ap_address, snonce, eapol + 17, ptk);
        state = WPA2_WAIT_M3;
        if (send_eapol(0x010a, 1, snonce) != 0) return C6_WPA2_FAILED;
        return C6_WPA2_EAPOL_M2;
    }
    if (state == WPA2_WAIT_M3 && length >= 131u &&
        same(frame + 24, eapol_llc, sizeof(eapol_llc))) {
        const uint8_t *eapol = frame + 32;
        if (eapol[5] != 3 || eapol[6] != 0xca || eapol[16] != 2 ||
            !verify_m3(eapol)) return C6_WPA2_FAILED;
        c6_mac_install_ccmp(ap_address, ptk + 32);
        if (send_eapol(0x030a, 2, (const uint8_t[32]){0}) != 0)
            return C6_WPA2_FAILED;
        if (send_ccmp_ping() != 0) return C6_WPA2_FAILED;
        state = WPA2_WAIT_CCMP;
        return C6_WPA2_EAPOL_M4 | C6_WPA2_CCMP_INSTALLED | C6_WPA2_CCMP_TX;
    }
    if (state == WPA2_WAIT_CCMP && length >= 52u && frame[1] == 0x42u &&
        same(frame + 4, station_address, 6)) {
        static const uint8_t pong[] = {
            0xaa,0xaa,3,0,0,0,0x88,0xb5,'P','O','N','G'
        };
        if (same(frame + 32, pong, sizeof(pong))) return C6_WPA2_CCMP_RX;
    }
    return C6_WPA2_NONE;
}
