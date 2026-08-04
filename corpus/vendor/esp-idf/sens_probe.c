#include <stdint.h>

#include "soc/rtc_i2c_reg.h"
#include "soc/sens_reg.h"

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    if (READ32(SENS_SAR_READER1_CTRL_REG) != 0x20040002u ||
        READ32(SENS_SAR_ATTEN1_REG) != 0xffffffffu ||
        READ32(SENS_SAR_TOUCH_CONF_REG) != 0xfff07fffu ||
        READ32(SENS_SAR_HALL_CTRL_REG) != 0xa0000000u ||
        READ32(SENS_SARDATE_REG) != 0x02101180u) {
        failure = 1;
    }

    WRITE32(SENS_SAR_TOUCH_THRES3_REG, 200u);
    WRITE32(SENS_SAR_COCPU_INT_ENA_W1TS_REG,
            SENS_COCPU_TOUCH_DONE_INT_ENA |
                SENS_COCPU_TOUCH_ACTIVE_INT_ENA |
                SENS_COCPU_TOUCH_SCAN_DONE_INT_ENA);
    WRITE32(SENS_SAR_TOUCH_CONF_REG, SENS_TOUCH_OUTEN);
    if ((READ32(SENS_SAR_TOUCH_CHN_ST_REG) &
         (SENS_TOUCH_MEAS_DONE | BIT(3))) !=
            (SENS_TOUCH_MEAS_DONE | BIT(3)) ||
        (READ32(SENS_SAR_TOUCH_STATUS3_REG) & SENS_TOUCH_PAD3_DATA_M) != 0u ||
        (READ32(SENS_SAR_COCPU_INT_ST_REG) &
         (SENS_COCPU_TOUCH_DONE_INT_ST |
          SENS_COCPU_TOUCH_ACTIVE_INT_ST |
          SENS_COCPU_TOUCH_SCAN_DONE_INT_ST)) == 0u) {
        failure = 2;
    }
    WRITE32(SENS_SAR_COCPU_INT_CLR_REG, 0xfffu);
    WRITE32(SENS_SAR_COCPU_INT_ENA_W1TC_REG, 0xfffu);
    if (READ32(SENS_SAR_COCPU_INT_ST_REG) != 0u ||
        READ32(SENS_SAR_COCPU_INT_ENA_REG) != 0u) {
        failure = 3;
    }

    WRITE32(RTC_I2C_INT_ENA_REG, RTC_I2C_RX_DATA_INT_ENA);
    WRITE32(SENS_SAR_I2C_CTRL_REG,
            SENS_SAR_I2C_START_FORCE | SENS_SAR_I2C_START |
                (7u << 11) | 0x42u);
    if ((READ32(RTC_I2C_DATA_REG) & RTC_I2C_I2C_DONE) == 0u ||
        (READ32(RTC_I2C_DATA_REG) & RTC_I2C_I2C_RDATA_M) != 0xffu ||
        (READ32(RTC_I2C_INT_ST_REG) & RTC_I2C_RX_DATA_INT_ST) == 0u) {
        failure = 4;
    }

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
