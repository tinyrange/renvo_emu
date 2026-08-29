typedef unsigned int u32;
#define REG32(address) (*(volatile u32 *)(address))

#define RCC_APB2ENR REG32(0x40021018u)
#define RCC_APB1ENR REG32(0x4002101cu)
#define GPIOA_CRL REG32(0x40010800u)
#define GPIOA_IDR REG32(0x40010808u)
#define GPIOA_BSRR REG32(0x40010810u)
#define TIM2_CR1 REG32(0x40000000u)
#define TIM2_DIER REG32(0x4000000cu)
#define TIM2_SR REG32(0x40000010u)
#define TIM2_ARR REG32(0x4000002cu)
#define USART1_SR REG32(0x40013800u)
#define USART1_DR REG32(0x40013804u)
#define USART1_BRR REG32(0x40013808u)
#define USART1_CR1 REG32(0x4001380cu)

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((USART1_SR & (1u << 7)) == 0u) {
        }
        USART1_DR = (unsigned char)*text++;
    }
}

int main(void)
{
    u32 failures = 0;
    RCC_APB2ENR |= (1u << 0) | (1u << 2) | (1u << 14);
    RCC_APB1ENR |= 1u;
    GPIOA_CRL = (GPIOA_CRL & ~(0xfu << 20)) | (1u << 20);
    GPIOA_BSRR = 1u << 5;
    USART1_BRR = 0x271u;
    USART1_CR1 = (1u << 13) | (1u << 3);
    uart_write("STM32F103\n");
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
