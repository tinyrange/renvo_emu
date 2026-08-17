/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_WPA2_H
#define REMU_C6_WPA2_H

#include <stdint.h>

enum c6_wpa2_event {
    C6_WPA2_NONE = 0,
    C6_WPA2_SCANNED = 1u << 0,
    C6_WPA2_AUTHENTICATED = 1u << 1,
    C6_WPA2_ASSOCIATED = 1u << 2,
    C6_WPA2_EAPOL_M2 = 1u << 3,
    C6_WPA2_EAPOL_M4 = 1u << 4,
    C6_WPA2_CCMP_INSTALLED = 1u << 5,
    C6_WPA2_CCMP_TX = 1u << 6,
    C6_WPA2_CCMP_RX = 1u << 7,
    C6_WPA2_FAILED = 1u << 8,
};

int c6_wpa2_start(void);
uint32_t c6_wpa2_receive(const uint8_t *frame, uint32_t length);

#endif
