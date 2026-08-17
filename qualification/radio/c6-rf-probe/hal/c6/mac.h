/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_MAC_H
#define REMU_C6_MAC_H

#include <stdint.h>

enum c6_mac_result {
    C6_MAC_RX_FRAME = 1,
    C6_MAC_OK = 0,
    C6_MAC_NO_PACKET = 0,
    C6_MAC_NOT_READY = -1,
    C6_MAC_TIMEOUT = -2,
    C6_MAC_INVALID_FRAME = -3,
    C6_MAC_INVALID_RX_DESCRIPTOR = -4,
    C6_MAC_BUFFER_TOO_SMALL = -5,
};

int c6_mac_init(void);
int c6_mac_tx_probe(const char *tag);
int c6_mac_tx_frame(const uint8_t *frame, uint32_t length);
void c6_mac_rx_start(void);
void c6_mac_rx_stop(void);
int c6_mac_rx_poll(uint32_t *wire_length, int8_t *rssi);
int c6_mac_rx_copy(uint8_t *frame, uint32_t capacity,
                   uint32_t *length, int8_t *rssi);
void c6_mac_set_interface_address(const uint8_t address[6]);
void c6_mac_install_ccmp(const uint8_t peer[6], const uint8_t key[16]);

#endif
