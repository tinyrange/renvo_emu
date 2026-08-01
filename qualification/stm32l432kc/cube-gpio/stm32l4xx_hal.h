#ifndef RENVO_STM32L4XX_HAL_H
#define RENVO_STM32L4XX_HAL_H

typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

typedef struct {
    volatile uint32_t MODER;
    volatile uint32_t OTYPER;
    volatile uint32_t OSPEEDR;
    volatile uint32_t PUPDR;
    volatile uint32_t IDR;
    volatile uint32_t ODR;
    volatile uint32_t BSRR;
} GPIO_TypeDef;

typedef struct { uint32_t Pin; uint32_t Mode; uint32_t Pull; uint32_t Speed; } GPIO_InitTypeDef;
typedef struct { uint32_t PLLState; uint32_t PLLSource; uint32_t PLLM; uint32_t PLLN; uint32_t PLLR; uint32_t PLLP; uint32_t PLLQ; } RCC_PLLInitTypeDef;
typedef struct { uint32_t OscillatorType; uint32_t MSIState; uint32_t MSIClockRange; uint32_t MSICalibrationValue; RCC_PLLInitTypeDef PLL; } RCC_OscInitTypeDef;
typedef struct { uint32_t ClockType; uint32_t SYSCLKSource; uint32_t AHBCLKDivider; uint32_t APB1CLKDivider; uint32_t APB2CLKDivider; } RCC_ClkInitTypeDef;

#define GPIOB ((GPIO_TypeDef *)0x48000400u)
#define GPIO_PIN_3 (1u << 3)
#define RCC_AHB2ENR (*(volatile uint32_t *)0x4002104cu)
#define GPIO_MODE_OUTPUT_PP 1u
#define GPIO_PULLUP 1u
#define GPIO_SPEED_FREQ_VERY_HIGH 3u
#define RCC_OSCILLATORTYPE_MSI 1u
#define RCC_MSI_ON 1u
#define RCC_MSIRANGE_6 6u
#define RCC_MSICALIBRATION_DEFAULT 0u
#define RCC_PLL_ON 1u
#define RCC_PLLSOURCE_MSI 1u
#define RCC_CLOCKTYPE_SYSCLK 1u
#define RCC_CLOCKTYPE_HCLK 2u
#define RCC_CLOCKTYPE_PCLK1 4u
#define RCC_CLOCKTYPE_PCLK2 8u
#define RCC_SYSCLKSOURCE_PLLCLK 3u
#define RCC_SYSCLK_DIV1 1u
#define RCC_HCLK_DIV1 1u
#define FLASH_LATENCY_4 4u
#define HAL_OK 0

void HAL_Init(void);
int HAL_RCC_OscConfig(const RCC_OscInitTypeDef *configuration);
int HAL_RCC_ClockConfig(const RCC_ClkInitTypeDef *configuration, uint32_t latency);
void HAL_GPIO_Init(GPIO_TypeDef *port, const GPIO_InitTypeDef *configuration);
void HAL_GPIO_TogglePin(GPIO_TypeDef *port, uint32_t pin);
void HAL_Delay(uint32_t milliseconds);

#endif
