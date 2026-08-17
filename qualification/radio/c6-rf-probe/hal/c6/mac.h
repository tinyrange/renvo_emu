/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_MAC_H
#define REMU_C6_MAC_H

#include <stdint.h>

int c6_mac_init(void);
int c6_mac_tx_probe(const char *tag);
void c6_mac_rx_start(void);
void c6_mac_rx_stop(void);
int c6_mac_rx_poll(uint32_t *wire_length, int8_t *rssi);

#endif
