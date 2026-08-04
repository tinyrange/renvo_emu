#include <stdint.h>

#include "soc/rtc_i2c_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    if (READ32(RTC_I2C_SCL_LOW_REG) != 0x100u ||
        READ32(RTC_I2C_CTRL_REG) != 0u ||
        READ32(RTC_I2C_DATE_REG) != 0x01905310u) {
        failure = 1;
    }

    WRITE32(RTC_I2C_SLAVE_ADDR_REG, 0x42u);
    WRITE32(RTC_I2C_DATA_REG, 0xa5u << RTC_I2C_SLAVE_TX_DATA_S);
    WRITE32(RTC_I2C_INT_ENA_REG,
            RTC_I2C_MASTER_TRAN_COMP_INT_ENA |
                RTC_I2C_TRANS_COMPLETE_INT_ENA |
                RTC_I2C_TX_DATA_INT_ENA);
    WRITE32(RTC_I2C_CTRL_REG,
            RTC_I2C_I2CCLK_EN | RTC_I2C_I2C_CTRL_CLK_GATE_EN |
                RTC_I2C_MS_MODE | RTC_I2C_TRANS_START);
    if ((READ32(RTC_I2C_CTRL_REG) & RTC_I2C_TRANS_START) != 0u ||
        (READ32(RTC_I2C_DATA_REG) & RTC_I2C_I2C_DONE) == 0u ||
        (READ32(RTC_I2C_CMD0_REG) & RTC_I2C_COMMAND0_DONE) == 0u ||
        (READ32(RTC_I2C_INT_ST_REG) &
         (RTC_I2C_MASTER_TRAN_COMP_INT_ST |
          RTC_I2C_TRANS_COMPLETE_INT_ST |
          RTC_I2C_TX_DATA_INT_ST)) == 0u) {
        failure = 2;
    }

    WRITE32(RTC_I2C_INT_CLR_REG, 0x1ffu);
    if (READ32(RTC_I2C_INT_RAW_REG) != 0u ||
        READ32(RTC_I2C_INT_ST_REG) != 0u) {
        failure = 3;
    }

    WRITE32(RTC_I2C_CTRL_REG, RTC_I2C_I2C_RESET);
    if (READ32(RTC_I2C_CTRL_REG) != 0u ||
        READ32(RTC_I2C_SCL_LOW_REG) != 0x100u ||
        READ32(RTC_I2C_DATE_REG) != 0x01905310u) {
        failure = 4;
    }

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
