#ifndef UART_BASE
#error "UART_BASE must select the target UART0 address"
#endif

#define UART_DR (*(volatile unsigned int *)(UART_BASE + 0x00u))
#define UART_FR (*(volatile unsigned int *)(UART_BASE + 0x18u))
#define UART_CR (*(volatile unsigned int *)(UART_BASE + 0x30u))

int main(void)
{
    static const unsigned char message[] = "REMU-RP\n";
    UART_CR = (1u << 8) | 1u;
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        while ((UART_FR & (1u << 5)) != 0u) {
        }
        UART_DR = message[index];
    }
    return 0;
}
