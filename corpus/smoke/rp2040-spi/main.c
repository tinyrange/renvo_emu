#define REG32(address) (*(volatile unsigned int *)(address))

#define SPI0_BASE 0x4003c000u
#define SPI1_BASE 0x40040000u
#define CTRL0 0x00u
#define SSIENR 0x08u
#define SER 0x10u
#define RXFLR 0x24u
#define SR 0x28u
#define IMR 0x2cu
#define ISR 0x30u
#define DR0 0x60u
#define TFE (1u << 2)
#define TFNF (1u << 1)
#define RFNE (1u << 3)
#define RXFIM (1u << 4)

static int check_spi(unsigned int base, unsigned int expected)
{
    REG32(base + CTRL0) = 7u;
    REG32(base + SER) = 1u;
    REG32(base + SSIENR) = 1u;
    if ((REG32(base + SR) & (TFE | TFNF)) != (TFE | TFNF)) {
        return 1;
    }
    REG32(base + IMR) = RXFIM;
    REG32(base + DR0) = expected;
    if ((REG32(base + RXFLR) & 0xffu) != 1u) {
        return 2;
    }
    if ((REG32(base + SR) & RFNE) == 0u) {
        return 3;
    }
    if ((REG32(base + ISR) & RXFIM) == 0u) {
        return 4;
    }
    if (REG32(base + DR0) != expected) {
        return 5;
    }
    return 0;
}

int main(void)
{
    int result = check_spi(SPI0_BASE, 0x5au);
    if (result != 0) {
        return result;
    }
    return check_spi(SPI1_BASE, 0xa5u);
}
