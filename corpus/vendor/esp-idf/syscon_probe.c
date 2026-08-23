#include <stdint.h>

#include "soc/syscon_reg.h"

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    volatile uint32_t *const lock = (volatile uint32_t *)SYSCON_EXT_MEM_PMS_LOCK_REG;
    volatile uint32_t *const attribute = (volatile uint32_t *)SYSCON_FLASH_ACE0_ATTR_REG;
    volatile uint32_t *const radio_clock = (volatile uint32_t *)SYSCON_WIFI_CLK_EN_REG;
    volatile uint32_t *const radio_reset = (volatile uint32_t *)SYSCON_WIFI_RST_EN_REG;
    const uint32_t tick_reset = SYSCON_TICK_ENABLE |
                                (7u << SYSCON_CK8M_TICK_NUM_S) |
                                (39u << SYSCON_XTAL_TICK_NUM_S);

    if (*(volatile uint32_t *)SYSCON_DATE_REG != 0x02101150u) {
        *exit_code = 1;
    } else if (*(volatile uint32_t *)SYSCON_TICK_CONF_REG != tick_reset) {
        *exit_code = 2;
    } else if (*radio_clock != 0xfffce030u || *radio_reset != 0) {
        *exit_code = 3;
    } else if (*(volatile uint32_t *)SYSCON_SPI_MEM_PMS_CTRL_REG != 0) {
        *exit_code = 4;
    } else {
        *radio_reset = 1u;
        *radio_clock = 0u;
        if (*radio_reset != 1u || *radio_clock != 0u) {
            *exit_code = 5;
            __asm__ volatile("break 0, 0");
            for (;;) {
            }
        }
        *radio_reset = 0u;
        *radio_clock = 0xfffce030u;
        *attribute = 0x12au;
        *lock = SYSCON_EXT_MEM_PMS_LOCK;
        *attribute = 0x55u;
        *lock = 0;
        if (*attribute != 0x12au || *lock != SYSCON_EXT_MEM_PMS_LOCK) {
            *exit_code = 6;
        } else {
            *exit_code = 0;
        }
    }
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
