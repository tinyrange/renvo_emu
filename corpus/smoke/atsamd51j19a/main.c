typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
#define REG8(address) (*(volatile u8 *)(address))
#define REG16(address) (*(volatile u16 *)(address))
#define REG32(address) (*(volatile u32 *)(address))

#define MCLK_APBAMASK REG32(0x40000814u)
#define PORTA_DIRSET REG32(0x41008008u)
#define PORTA_OUTCLR REG32(0x41008014u)
#define PORTA_OUTSET REG32(0x41008018u)
#define PORTA_IN REG32(0x41008020u)
#define SERCOM0_CTRLA REG32(0x40003000u)
#define SERCOM0_INTFLAG REG8(0x40003018u)
#define SERCOM0_DATA REG8(0x40003028u)
#define TC0_CTRLA REG32(0x40003800u)
#define TC0_INTFLAG REG8(0x4000380au)
#define TC0_CC0 REG16(0x4000381cu)

static void uart_write(const char *text)
{
    SERCOM0_CTRLA = 1u << 1;
    while (*text != '\0') {
        while ((SERCOM0_INTFLAG & 1u) == 0u) {
        }
        SERCOM0_DATA = (u8)*text++;
    }
}

int main(void)
{
    u32 failures = 0;
    MCLK_APBAMASK |= 1u;
    PORTA_DIRSET = 1u << 13;
    PORTA_OUTSET = 1u << 13;
    uart_write("SAMD51J19A\n");
    TC0_CC0 = 8;
    TC0_INTFLAG = 1u << 4;
    TC0_CTRLA = 1u << 1;
    while ((TC0_INTFLAG & (1u << 4)) == 0u) {
    }
    TC0_CTRLA = 0;
    failures |= (PORTA_IN & (1u << 3)) == 0u;
    PORTA_OUTCLR = 1u << 13;
    return (int)failures;
}
