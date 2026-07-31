#define REG32(address) (*(volatile unsigned int *)(address))

#define SIO_GPIO_OUT_SET REG32(0xd0000014u)
#define SIO_GPIO_OE_SET REG32(0xd0000024u)

int main(void)
{
    SIO_GPIO_OE_SET = 1u << 25;
    SIO_GPIO_OUT_SET = 1u << 25;
    return 0;
}
