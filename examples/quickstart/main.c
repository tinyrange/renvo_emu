#define REG32(address) (*(volatile unsigned int *)(address))

#define RCC_APB2PCENR REG32(0x40021018u)
#define GPIOC_CFGLR REG32(0x40011000u)
#define GPIOC_BSHR REG32(0x40011010u)

int main(void)
{
    RCC_APB2PCENR |= 1u << 4;
    GPIOC_CFGLR = (GPIOC_CFGLR & ~(0xfu << 4)) | (1u << 4);
    GPIOC_BSHR = 1u << 1;
    return 0;
}
