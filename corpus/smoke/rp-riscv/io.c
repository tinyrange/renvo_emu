#define REG32(address) (*(volatile unsigned int *)(address))

#define SIO_GPIO_OUT_SET REG32(0xd0000018u)
#define SIO_GPIO_OE_SET REG32(0xd0000038u)
#define GPIO0_STATUS REG32(0x40028000u)
#define GPIO0_CTRL REG32(0x40028004u)
#define INTR0 REG32(0x40028230u)
#define PROC1_INTE0 REG32(0x40028290u)
#define PROC1_INTF0 REG32(0x400282a8u)
#define PROC1_INTS0 REG32(0x400282c0u)

int main(void)
{
    const unsigned int event = 1u << 3;
    SIO_GPIO_OE_SET = 1u;
    SIO_GPIO_OUT_SET = 1u;
    GPIO0_CTRL = (3u << 28) | (3u << 16) | (3u << 14) | (3u << 12) | 5u;
    if (GPIO0_CTRL != 0x3003f005u) {
        return 1;
    }
    if ((GPIO0_STATUS & ((1u << 9) | (1u << 13))) != ((1u << 9) | (1u << 13))) {
        return 2;
    }
    INTR0 = event;
    PROC1_INTE0 = event;
    PROC1_INTF0 = event;
    if ((PROC1_INTS0 & event) == 0) {
        return 3;
    }
    PROC1_INTF0 = 0;
    return (PROC1_INTS0 & event) == 0 ? 0 : 4;
}
