#define REG32(address) (*(volatile unsigned int *)(address))

#define RCC_APB2PCENR REG32(0x40021018u)
#define USART1_STATR REG32(0x40013800u)
#define USART1_DATAR REG32(0x40013804u)
#define USART1_BRR REG32(0x40013808u)
#define USART1_CTLR1 REG32(0x4001380cu)

static void putc(unsigned char byte)
{
    while ((USART1_STATR & (1u << 7)) == 0u) {
    }
    USART1_DATAR = byte;
}

int main(void)
{
    static const unsigned char message[] = "REMU-WCH\n";
    RCC_APB2PCENR |= 1u << 14;
    USART1_BRR = 0x01a1u;
    USART1_CTLR1 = (1u << 13) | (1u << 3);
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        putc(message[index]);
    }
    return 0;
}
