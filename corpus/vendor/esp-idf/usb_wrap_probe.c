#include <stdint.h>

#include "soc/usb_wrap_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    volatile uint32_t *const otg = (volatile uint32_t *)USB_WRAP_OTG_CONF_REG;
    volatile uint32_t *const test = (volatile uint32_t *)USB_WRAP_TEST_CONF_REG;
    const uint32_t reset = USB_WRAP_PHY_CLK_FORCE_ON |
                           USB_WRAP_AHB_CLK_FORCE_ON |
                           USB_WRAP_USB_PAD_ENABLE;

    if (*(volatile uint32_t *)USB_WRAP_DATE_REG != 0x02102010u) {
        *exit_code = 1;
    } else if (*otg != reset || *test != 0) {
        *exit_code = 2;
    } else {
        *otg = USB_WRAP_USB_PAD_ENABLE | USB_WRAP_PAD_PULL_OVERRIDE |
               USB_WRAP_DP_PULLUP;
        *test = USB_WRAP_TEST_ENABLE | USB_WRAP_TEST_USB_OE |
                USB_WRAP_TEST_TX_DP;
        if (*otg != (USB_WRAP_USB_PAD_ENABLE | USB_WRAP_PAD_PULL_OVERRIDE |
                     USB_WRAP_DP_PULLUP)) {
            *exit_code = 3;
        } else if ((*test & 0x0fu) != 0x07u) {
            *exit_code = 4;
        } else {
            *exit_code = 0;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
