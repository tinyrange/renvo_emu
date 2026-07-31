#define REG32(address) (*(volatile unsigned int *)(address))

#define GPIO_OUT_W1TS REG32(0x60091008u)
#define GPIO_ENABLE_W1TS REG32(0x60091024u)

int main(void)
{
    GPIO_ENABLE_W1TS = 1u << 2;
    GPIO_OUT_W1TS = 1u << 2;
    return 0;
}
