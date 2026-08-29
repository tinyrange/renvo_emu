typedef unsigned int u32;
#define REG32(address) (*(volatile u32 *)(address))

#define P0_OUTSET REG32(0x50000508u)
#define P0_OUTCLR REG32(0x5000050cu)
#define P0_IN REG32(0x50000510u)
#define P0_DIRSET REG32(0x50000518u)
#define UART_STARTTX REG32(0x40002008u)
#define UART_EVENT_TXDRDY REG32(0x4000211cu)
#define UART_ENABLE REG32(0x40002500u)
#define UART_TXD REG32(0x4000251cu)
#define TIMER_START REG32(0x40008000u)
#define TIMER_EVENT_COMPARE0 REG32(0x40008140u)
#define TIMER_CC0 REG32(0x40008540u)

static void uart_write(const char *text)
{
    UART_ENABLE = 4;
    UART_STARTTX = 1;
    while (*text != '\0') {
        UART_EVENT_TXDRDY = 0;
        UART_TXD = (unsigned char)*text++;
        while (UART_EVENT_TXDRDY == 0u) {
        }
    }
}

int main(void)
{
    u32 failures = 0;
    P0_DIRSET = 1u << 13;
    P0_OUTSET = 1u << 13;
    uart_write("NRF52840\n");
    TIMER_CC0 = 8;
    TIMER_EVENT_COMPARE0 = 0;
    TIMER_START = 1;
    while (TIMER_EVENT_COMPARE0 == 0u) {
    }
    failures |= (P0_IN & (1u << 3)) == 0u;
    P0_OUTCLR = 1u << 13;
    return (int)failures;
}
