#define REG32(address) (*(volatile unsigned int *)(address))

#define SIO_GPIO_OUT_SET REG32(0xd0000014u)
#define SIO_GPIO_OE_SET REG32(0xd0000024u)
#define GPIO0_STATUS REG32(0x40014000u)
#define GPIO0_CTRL REG32(0x40014004u)
#define PROC0_INTE0 REG32(0x40014100u)
#define PROC0_INTF0 REG32(0x40014110u)
#define PROC0_INTS0 REG32(0x40014120u)

int main(void)
{
    const unsigned int event = 1u << 3;
    SIO_GPIO_OE_SET = 1u;
    SIO_GPIO_OUT_SET = 1u;
    GPIO0_CTRL = 5u;
    if (GPIO0_CTRL != 5u) {
        return 1;
    }
    const unsigned int output_status =
        (1u << 15) | (1u << 13) | (1u << 9) | (1u << 8);
    if ((GPIO0_STATUS & output_status) != output_status) {
        return 2;
    }
    PROC0_INTE0 = event;
    PROC0_INTF0 = event;
    if ((PROC0_INTS0 & event) == 0u) {
        return 3;
    }
    PROC0_INTF0 = 0u;
    return (PROC0_INTS0 & event) == 0u ? 0 : 4;
}
