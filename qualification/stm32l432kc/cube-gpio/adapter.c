#include "stm32l4xx_hal.h"

void HAL_Init(void) {}

int HAL_RCC_OscConfig(const RCC_OscInitTypeDef *configuration)
{
    (void)configuration;
    return HAL_OK;
}

int HAL_RCC_ClockConfig(const RCC_ClkInitTypeDef *configuration, uint32_t latency)
{
    (void)configuration;
    (void)latency;
    return HAL_OK;
}

void HAL_GPIO_Init(GPIO_TypeDef *port, const GPIO_InitTypeDef *configuration)
{
    for (uint32_t pin = 0; pin < 16; ++pin) {
        if ((configuration->Pin & (1u << pin)) != 0u) {
            port->MODER = (port->MODER & ~(3u << (pin * 2u))) | (1u << (pin * 2u));
        }
    }
}

void HAL_GPIO_TogglePin(GPIO_TypeDef *port, uint32_t pin)
{
    port->BSRR = (port->ODR & pin) != 0u ? pin << 16 : pin;
}

void HAL_Delay(uint32_t milliseconds)
{
    for (volatile uint32_t tick = 0; tick < milliseconds; ++tick) {
    }
}
