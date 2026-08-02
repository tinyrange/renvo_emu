#define UART0_FIFO (*(volatile unsigned int *)0x60000000u)
#define UART1_FIFO (*(volatile unsigned int *)0x60001000u)
#define LP_UART_FIFO (*(volatile unsigned int *)0x600b1400u)

int main(void)
{
    static const unsigned char uart0_message[] = "U0\n";
    static const unsigned char uart1_message[] = "U1\n";
    static const unsigned char lp_message[] = "LP\n";
    for (unsigned int index = 0; index < sizeof(uart0_message) - 1u; ++index) {
        UART0_FIFO = uart0_message[index];
    }
    for (unsigned int index = 0; index < sizeof(uart1_message) - 1u; ++index) {
        UART1_FIFO = uart1_message[index];
    }
    for (unsigned int index = 0; index < sizeof(lp_message) - 1u; ++index) {
        LP_UART_FIFO = lp_message[index];
    }
    return 0;
}
