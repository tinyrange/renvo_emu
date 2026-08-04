#include <stdint.h>

#include "soc/uhci_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    const uint32_t conf0_reset = UHCI_ENCODE_CRC_EN | UHCI_LEN_EOF_EN |
                                 UHCI_CRC_REC_EN | UHCI_HEAD_EN | UHCI_SEPER_EN;

    if (*(volatile uint32_t *)UHCI_DATE_REG(0) != 0x02010090u) {
        *exit_code = 1;
    } else if (*(volatile uint32_t *)UHCI_CONF0_REG(0) != conf0_reset) {
        *exit_code = 2;
    } else {
        *(volatile uint32_t *)UHCI_CONF0_REG(0) = conf0_reset | UHCI_UART2_CE;
        *(volatile uint32_t *)UHCI_INT_ENA_REG(0) = UHCI_APP_CTRL0_INT_ENA;
        *(volatile uint32_t *)UHCI_APP_INT_SET_REG(0) = UHCI_APP_CTRL0_INT_SET;
        if ((*(volatile uint32_t *)UHCI_INT_ST_REG(0) & UHCI_APP_CTRL0_INT_ST) == 0) {
            *exit_code = 3;
        } else {
            *(volatile uint32_t *)UHCI_INT_CLR_REG(0) = UHCI_APP_CTRL0_INT_CLR;
            *exit_code = (*(volatile uint32_t *)UHCI_INT_ST_REG(0) == 0) ? 0 : 4;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
