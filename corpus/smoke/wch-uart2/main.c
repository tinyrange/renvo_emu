#define REG32(address) (*(volatile unsigned int *)(address))

#define RCC_APB2PCENR REG32(0x40021018u)
#define USART2_STATR REG32(0x40004400u)
#define USART2_DATAR REG32(0x40004404u)
#define USART2_BRR REG32(0x40004408u)
#define USART2_CTLR1 REG32(0x4000440cu)

static void putc(unsigned char byte)
{
    while ((USART2_STATR & (1u << 7)) == 0u) {
    }
    USART2_DATAR = byte;
}

int main(void)
{
    static const unsigned char message[] = "REMU-WCH2\n";
    RCC_APB2PCENR |= 1u << 13;
    USART2_BRR = 0x01a1u;
    USART2_CTLR1 = (1u << 13) | (1u << 3);
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        putc(message[index]);
    }
    return 0;
}
