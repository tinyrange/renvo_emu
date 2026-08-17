/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_MAC_H
#define REMU_C6_MAC_H

#include <stdint.h>

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
