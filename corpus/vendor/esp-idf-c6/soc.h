#pragma once

#include "soc/reg_base.h"

#define BIT(number) (1U << (number))
#define REG_IO_MUX_BASE DR_REG_IO_MUX_BASE
#define REG_I2C_BASE(instance) DR_REG_I2C_EXT_BASE
#define REG_MCPWM_BASE(instance) DR_REG_MCPWM_BASE
#define REG_SPI_BASE(instance) DR_REG_SPI2_BASE
#define REG_TIMG_BASE(instance) \
    ((instance) == 0 ? DR_REG_TIMERGROUP0_BASE : DR_REG_TIMERGROUP1_BASE)
#define REG_UHCI_BASE(instance) DR_REG_UHCI0_BASE
