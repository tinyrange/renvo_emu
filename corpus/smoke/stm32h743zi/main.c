typedef unsigned int u32;
#define REG32(address) (*(volatile u32 *)(address))

#define RCC_CR REG32(0x58024400u)
#define RCC_AHB1ENR REG32(0x580244d8u)
#define RCC_AHB4ENR REG32(0x580244e0u)
#define RCC_APB1LENR REG32(0x580244e8u)
#define FLASH_ACR REG32(0x52002000u)
#define GPIOA_MODER REG32(0x58020000u)
#define GPIOA_IDR REG32(0x58020010u)
#define GPIOA_BSRR REG32(0x58020018u)
#define TIM2_CR1 REG32(0x40000000u)
#define TIM2_DIER REG32(0x4000000cu)
#define TIM2_SR REG32(0x40000010u)
#define TIM2_ARR REG32(0x4000002cu)
#define USART3_CR1 REG32(0x40004800u)
#define USART3_ISR REG32(0x4000481cu)
#define USART3_TDR REG32(0x40004828u)
#define DMA1_LISR REG32(0x40020000u)
#define DMA1_LIFCR REG32(0x40020008u)
#define DMA1_S0CR REG32(0x40020010u)
#define DMA1_S0NDTR REG32(0x40020014u)
#define DMA1_S0PAR REG32(0x40020018u)
#define DMA1_S0M0AR REG32(0x4002001cu)

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((USART3_ISR & (1u << 7)) == 0u) {
        }
        USART3_TDR = (unsigned char)*text++;
    }
}

int main(void)
{
    volatile u32 *source = (volatile u32 *)0x24000000u;
    volatile u32 *destination = (volatile u32 *)0x24000008u;
    u32 failures = 0;

    failures |= (RCC_CR & 3u) != 3u;
    RCC_AHB1ENR |= 3u;
    RCC_AHB4ENR |= 1u;
    RCC_APB1LENR |= (1u << 0) | (1u << 18);
    FLASH_ACR = 3u;
    failures |= (FLASH_ACR & 0x3fu) != 3u;

    GPIOA_MODER = (GPIOA_MODER & ~((3u << 10) | (3u << 6))) | (1u << 10);
    GPIOA_BSRR = 1u << 5;
    USART3_CR1 = (1u << 0) | (1u << 3);
    uart_write("STM32H743\n");

    source[0] = 0x12345678u;
    source[1] = 0x9abcdef0u;
    destination[0] = 0u;
    destination[1] = 0u;
    DMA1_LIFCR = 0x3fu;
    DMA1_S0PAR = (u32)source;
    DMA1_S0M0AR = (u32)destination;
    DMA1_S0NDTR = 2u;
    DMA1_S0CR = 1u | (1u << 4) | (2u << 6) | (1u << 9) | (1u << 10)
        | (2u << 11) | (2u << 13);
    while ((DMA1_LISR & (1u << 5)) == 0u) {
    }
    failures |= destination[0] != source[0];
    failures |= destination[1] != source[1];

    TIM2_ARR = 8u;
    TIM2_DIER = 1u;
    TIM2_CR1 = 1u;
    while ((TIM2_SR & 1u) == 0u) {
    }
    TIM2_CR1 = 0u;
    failures |= (GPIOA_IDR & (1u << 3)) == 0u;
    GPIOA_BSRR = 1u << 21;
    return (int)failures;
}
