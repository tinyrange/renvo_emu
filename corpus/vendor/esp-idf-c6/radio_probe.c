#include <stdint.h>

#include "soc/ieee802154_reg.h"
#include "modem/modem_lpcon_reg.h"
#include "modem/modem_syscon_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

static uint8_t tx_frame[] = {4, 0x01, 0x00, 0x2a, 0xa5};

int main(void)
{
    if ((READ32(IEEE802154_MAC_DATE_REG) & IEEE802154_MAC_DATE) != 2229794u) {
        return 1;
    }
    if ((READ32(MODEM_SYSCON_DATE_REG) & MODEM_SYSCON_DATE) != 35676928u) {
        return 2;
    }
    if ((READ32(MODEM_LPCON_DATE_REG) & MODEM_LPCON_DATE) != 35676736u) {
        return 3;
    }

    WRITE32(MODEM_SYSCON_CLK_CONF_REG,
            MODEM_SYSCON_CLK_ZB_APB_EN | MODEM_SYSCON_CLK_ZB_MAC_EN |
            MODEM_SYSCON_CLK_MODEM_SEC_CCM_EN);
    WRITE32(IEEE802154_CHANNEL_REG, 11u);
    WRITE32(IEEE802154_CTRL_CFG_REG,
            IEEE802154_PROMISCUOUS_MODE | IEEE802154_HW_AUTO_ACK_RX_EN);
    WRITE32(IEEE802154_EVENT_EN_REG, 0x1fffu);
    WRITE32(IEEE802154_TXDMA_ADDR_REG, (uintptr_t)tx_frame);
    WRITE32(IEEE802154_COMMAND_REG, 0x41u);

    for (uint32_t timeout = 0; timeout < 1000u; ++timeout) {
        if ((READ32(IEEE802154_EVENT_STATUS_REG) & 1u) != 0) {
            WRITE32(IEEE802154_EVENT_STATUS_REG, 1u);
            if ((READ32(IEEE802154_EVENT_STATUS_REG) & 1u) != 0) {
                return 4;
            }
            WRITE32(IEEE802154_COMMAND_REG, 0x44u);
            for (uint32_t ed_timeout = 0; ed_timeout < 1000u; ++ed_timeout) {
                if ((READ32(IEEE802154_EVENT_STATUS_REG) & (1u << 6)) != 0) {
                    return 0;
                }
            }
            return 5;
        }
    }
    return 6;
}
