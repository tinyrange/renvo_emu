/* SPDX-License-Identifier: Apache-2.0 */
#include "mmio.h"
#include "phy.h"
#include "registers.h"
#include "rf.h"

#define MODEM_WIFI_APB_CLOCK (1u << 9)
#define MODEM_WIFI_MAC_CLOCK (1u << 10)
#define WIFI_RESET_START (1u << 1)
#define WIFI_RESET_READY (1u << 0)

int c6_phy_reset_radio(void)
{
    c6_rf_rx_enable(0);
    c6_write32(C6_REG_WIFI_RESET_CONTROL, WIFI_RESET_START);
    c6_rf_invalidate();
    return (c6_read32(C6_REG_WIFI_RESET_CONTROL) & WIFI_RESET_READY) != 0
               ? C6_PHY_OK
               : C6_PHY_RESET_NOT_READY;
}

int c6_phy_init(void)
{
    uint32_t clocks = c6_read32(C6_REG_MODEM_CLOCK_ENABLE);
    c6_write32(C6_REG_MODEM_CLOCK_ENABLE,
               clocks | MODEM_WIFI_APB_CLOCK | MODEM_WIFI_MAC_CLOCK);
    return c6_phy_reset_radio();
}
