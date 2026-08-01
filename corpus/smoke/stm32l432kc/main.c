typedef unsigned int u32;

#define REG32(address) (*(volatile u32 *)(address))
#define RCC_AHB2ENR REG32(0x4002104cu)
#define RCC_APB1ENR1 REG32(0x40021058u)
#define GPIOA_MODER REG32(0x48000000u)
#define GPIOA_BSRR REG32(0x48000018u)
#define GPIOA_IDR REG32(0x48000010u)
#define TIM2_CR1 REG32(0x40000000u)
#define TIM2_DIER REG32(0x4000000cu)
#define TIM2_SR REG32(0x40000010u)
#define TIM2_ARR REG32(0x4000002cu)
#define USART2_CR1 REG32(0x40004400u)
#define USART2_ISR REG32(0x4000441cu)
#define USART2_TDR REG32(0x40004428u)
#define NVIC_ISER0 REG32(0xe000e100u)

static volatile u32 timer_interrupts;
static volatile u32 dividend = 100000u;
static volatile u32 switch_input = 7u;

struct abi_record {
    unsigned char tag;
    unsigned short value;
    u32 wide;
};

enum abi_state {
    ABI_ZERO,
    ABI_LARGE = 300
};

static u32 recursive_sum(u32 value)
{
    return value == 0u ? 0u : value + recursive_sum(value - 1u);
}

static u32 switch_value(u32 value)
{
    switch (value) {
    case 0: return 11u;
    case 2: return 29u;
    case 7: return 47u;
    case 19: return 83u;
    default: return 101u;
    }
}

void tim2_handler(void)
{
    TIM2_SR = 0u;
    TIM2_CR1 = 0u;
    ++timer_interrupts;
}

static void uart_write(const char *text)
{
    while (*text != '\0') {
        while ((USART2_ISR & (1u << 7)) == 0u) {
        }
        USART2_TDR = (unsigned char)*text++;
    }
}

int main(void)
{
    u32 failures = 0u;
    struct abi_record record = { 9u, 0x1234u, 0x89abcdefu };
    enum abi_state state = ABI_LARGE;
    volatile float left = 3.25f;
    volatile float right = 2.0f;
    volatile float result = left * right;
    if ((int)(result * 4.0f) != 26) {
        failures |= 1u << 0;
    }
    failures |= (u32)((sizeof(char) != 1u) << 1);
    failures |= (u32)((sizeof(short) != 2u) << 2);
    failures |= (u32)((sizeof(int) != 4u) << 3);
    failures |= (u32)((sizeof(long) != 4u) << 4);
    failures |= (u32)((sizeof(void *) != 4u) << 5);
    failures |= (u32)((recursive_sum(12u) != 78u) << 6);
    failures |= (u32)(((dividend / 37u) != 2702u) << 7);
    failures |= (u32)(((dividend % 37u) != 26u) << 8);
    failures |= (u32)((switch_value(switch_input) != 47u) << 9);
    failures |= (u32)((record.tag != 9u || record.value != 0x1234u ||
                       record.wide != 0x89abcdefu) << 10);
    failures |= (u32)(((int)state != 300) << 11);

    RCC_AHB2ENR |= 1u;
    RCC_APB1ENR1 |= (1u << 0) | (1u << 17);
    GPIOA_MODER = (GPIOA_MODER & ~((3u << 10) | (3u << 6))) | (1u << 10);
    GPIOA_BSRR = 1u << 5;

    USART2_CR1 = (1u << 0) | (1u << 3);
    uart_write("STM32L432\n");

    TIM2_ARR = 8u;
    TIM2_DIER = 1u;
    NVIC_ISER0 = 1u << 28;
    TIM2_CR1 = 1u;
    while (timer_interrupts == 0u) {
    }
    if ((GPIOA_IDR & (1u << 3)) == 0u) {
        failures |= 1u << 12;
    }
    GPIOA_BSRR = 1u << (5u + 16u);
    return (int)failures;
}
