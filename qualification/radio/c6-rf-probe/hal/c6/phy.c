/* SPDX-License-Identifier: Apache-2.0 */
#include "mmio.h"
#include "phy.h"
#include "rf.h"

#define MODEM_SYSCON_WIFI_CLOCK_ENABLE 0x600a9814u
#define MODEM_WIFI_APB_CLOCK (1u << 9)
#define MODEM_WIFI_MAC_CLOCK (1u << 10)
#define WIFI_RESET_CONTROL 0x600a4ddcu
#define WIFI_RESET_START (1u << 1)
#define WIFI_RESET_READY (1u << 0)

int c6_phy_reset_radio(void)
{
    c6_rf_rx_enable(0);
    c6_write32(WIFI_RESET_CONTROL, WIFI_RESET_START);
    c6_rf_invalidate();
    return (c6_read32(WIFI_RESET_CONTROL) & WIFI_RESET_READY) != 0 ? 0 : -1;
}

int c6_phy_init(void)
{
    uint32_t clocks = c6_read32(MODEM_SYSCON_WIFI_CLOCK_ENABLE);
    c6_write32(MODEM_SYSCON_WIFI_CLOCK_ENABLE,
               clocks | MODEM_WIFI_APB_CLOCK | MODEM_WIFI_MAC_CLOCK);
    return c6_phy_reset_radio();
}
