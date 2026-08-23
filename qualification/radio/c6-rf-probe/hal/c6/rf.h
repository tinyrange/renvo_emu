/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_RF_H
#define REMU_C6_RF_H

#include <stdint.h>

enum c6_rf_result {
    C6_RF_OK = 0,
    C6_RF_INVALID_CHANNEL = -1,
    C6_RF_INVALID_POWER = -2,
    C6_RF_NOT_INITIALIZED = -3,
};

void c6_rf_invalidate(void);
int c6_rf_configure(uint8_t channel, uint8_t power_dbm);
int c6_rf_set_channel(uint8_t channel);
int c6_rf_set_power(uint8_t power_dbm);
void c6_rf_rx_enable(int enabled);
uint8_t c6_rf_channel(void);
uint8_t c6_rf_power_dbm(void);
int c6_rf_ready(void);

#endif
