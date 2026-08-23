/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_STATION_H
#define REMU_C6_STATION_H

#include <stdint.h>

enum c6_station_event {
    C6_STATION_NONE = 0,
    C6_STATION_SCANNED = 1u << 0,
    C6_STATION_AUTHENTICATED = 1u << 1,
    C6_STATION_ASSOCIATED = 1u << 2,
    C6_STATION_L2_TX = 1u << 3,
    C6_STATION_L2_RX = 1u << 4,
    C6_STATION_FAILED = 1u << 5,
};

int c6_station_start(void);
uint32_t c6_station_receive(const uint8_t *frame, uint32_t length);

#endif
