typedef unsigned int u32;
#define REG32(address) (*(volatile u32 *)(address))

#define GPIO_OUT_W1TS REG32(0x500e0008u)
#define GPIO_ENABLE REG32(0x500e0020u)
#define GPIO_IN REG32(0x500e003cu)
#define UART0_FIFO REG32(0x500ca000u)
#define TIMER0_CONFIG REG32(0x500c2000u)
#define TIMER0_COUNTER_LOW REG32(0x500c2004u)
#define TIMER0_UPDATE REG32(0x500c200cu)

static void uart_write(const char *text)
{
    while (*text != '\0') {
        UART0_FIFO = (unsigned char)*text++;
    }
}

int main(void)
{
    u32 failures = 0;
    GPIO_ENABLE = 1u << 5;
    GPIO_OUT_W1TS = 1u << 5;
    uart_write("ESP32P4\n");
    TIMER0_CONFIG = (1u << 31) | (1u << 30) | (2u << 13);
    do {
        TIMER0_UPDATE = 1;
    } while (TIMER0_COUNTER_LOW == 0u);
    failures |= (GPIO_IN & (1u << 3)) == 0u;
    return (int)failures;
}
