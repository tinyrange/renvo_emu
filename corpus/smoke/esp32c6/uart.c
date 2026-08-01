#define UART_FIFO (*(volatile unsigned int *)0x60000000u)

int main(void)
{
    static const unsigned char message[] = "REMU-ESP\n";
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        UART_FIFO = message[index];
    }
    return 0;
}
