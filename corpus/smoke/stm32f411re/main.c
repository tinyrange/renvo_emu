typedef unsigned int u32;
#define REG32(address) (*(volatile u32 *)(address))

#define RCC_AHB1ENR REG32(0x40023830u)
#define RCC_APB1ENR REG32(0x40023840u)
#define GPIOA_MODER REG32(0x40020000u)
#define GPIOA_IDR REG32(0x40020010u)
#define GPIOA_BSRR REG32(0x40020018u)
#define TIM2_CR1 REG32(0x40000000u)
#define TIM2_DIER REG32(0x4000000cu)
#define TIM2_SR REG32(0x40000010u)
#define TIM2_ARR REG32(0x4000002cu)
#define USART2_SR REG32(0x40004400u)
#define USART2_DR REG32(0x40004404u)
#define USART2_CR1 REG32(0x4000440cu)

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((USART2_SR & (1u << 7)) == 0u) {
        }
        USART2_DR = (unsigned char)*text++;
    }
}

int main(void)
{
    u32 failures = 0;
    RCC_AHB1ENR |= 1u;
    RCC_APB1ENR |= (1u << 0) | (1u << 17);
    GPIOA_MODER = (GPIOA_MODER & ~((3u << 10) | (3u << 6))) | (1u << 10);
    GPIOA_BSRR = 1u << 5;
    USART2_CR1 = (1u << 13) | (1u << 3);
    uart_write("STM32F411\n");
    TIM2_ARR = 8;
    TIM2_DIER = 1;
    TIM2_CR1 = 1;
    while ((TIM2_SR & 1u) == 0u) {
    }
    TIM2_CR1 = 0;
    failures |= (GPIOA_IDR & (1u << 3)) == 0u;
    GPIOA_BSRR = 1u << 21;
    return (int)failures;
}
