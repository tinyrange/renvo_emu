#include <stdint.h>

#include "soc/gpio_sd_reg.h"
#include "soc/rtc_io_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    if (READ32(RTC_GPIO_OUT_REG) != 0u ||
        READ32(RTC_IO_TOUCH_PAD0_REG) != 0x50000000u ||
        READ32(RTC_IO_TOUCH_PAD1_REG) != 0x48000000u ||
        READ32(RTC_IO_DATE_REG) != 0x02101180u) {
        failure = 1;
    }
    WRITE32(RTC_GPIO_OUT_REG, RTC_GPIO_OUT_DATA_M);
    WRITE32(RTC_GPIO_OUT_W1TC_REG, 1u << RTC_GPIO_OUT_DATA_S);
    if (READ32(RTC_GPIO_OUT_REG) !=
        (RTC_GPIO_OUT_DATA_M & ~(1u << RTC_GPIO_OUT_DATA_S))) {
        failure = 2;
    }
    WRITE32(RTC_GPIO_ENABLE_W1TS_REG, 3u << RTC_GPIO_ENABLE_W1TS_S);
    if (READ32(RTC_GPIO_ENABLE_REG) != (3u << RTC_GPIO_ENABLE_S)) {
        failure = 3;
    }
    WRITE32(RTC_GPIO_PIN0_REG, UINT32_MAX);
    if (READ32(RTC_GPIO_PIN0_REG) !=
        (RTC_GPIO_PIN0_WAKEUP_ENABLE_M | RTC_GPIO_PIN0_INT_TYPE_M |
         RTC_GPIO_PIN0_PAD_DRIVER_M)) {
        failure = 4;
    }

    if (READ32(GPIO_SIGMADELTA0_REG) != 0x0000ff00u ||
        READ32(GPIO_SIGMADELTA7_REG) != 0x0000ff00u ||
        READ32(GPIO_SIGMADELTA_VERSION_REG) != 0x01802260u) {
        failure = 5;
    }
    WRITE32(GPIO_SIGMADELTA0_REG, UINT32_MAX);
    WRITE32(GPIO_SIGMADELTA_MISC_REG, UINT32_MAX);
    if (READ32(GPIO_SIGMADELTA0_REG) !=
            (GPIO_SD0_PRESCALE_M | GPIO_SD0_IN_M) ||
        READ32(GPIO_SIGMADELTA_MISC_REG) !=
            (GPIO_SPI_SWAP_M | GPIO_FUNCTION_CLK_EN_M)) {
        failure = 6;
    }

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
