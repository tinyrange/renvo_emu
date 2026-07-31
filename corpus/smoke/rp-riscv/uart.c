#define UART_DR (*(volatile unsigned int *)0x40070000u)
#define UART_FR (*(volatile unsigned int *)0x40070018u)

int main(void)
{
    static const unsigned char message[] = "RENVO-RP\n";
    for (unsigned int index = 0; index < sizeof(message) - 1u; ++index) {
        while ((UART_FR & (1u << 5)) != 0u) {
        }
        UART_DR = message[index];
    }
    return 0;
}
