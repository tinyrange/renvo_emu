#ifndef REMU_STM32_CUBE_MAIN_H
#define REMU_STM32_CUBE_MAIN_H

#include "stm32l4xx_hal.h"

#define LED3_PIN GPIO_PIN_3
#define LED3_GPIO_PORT GPIOB
#define LED3_GPIO_CLK_ENABLE() (RCC_AHB2ENR |= 1u << 1)

#endif
