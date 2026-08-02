#define REG32(address) (*(volatile unsigned int *)(address))

#define SPI0_BASE 0x40080000u
#define SPI1_BASE 0x40088000u
#define CR0 0x00u
#define CR1 0x04u
#define DR 0x08u
#define SR 0x0cu
#define IMSC 0x14u
#define MIS 0x1cu
#define SSE (1u << 1)
#define LBM (1u << 0)
#define TFE (1u << 0)
#define TNF (1u << 1)
#define RNE (1u << 2)
#define TXIM (1u << 3)

static int check_spi(unsigned int base, unsigned int loopback, unsigned int expected)
{
    REG32(base + CR0) = 7u;
    REG32(base + CR1) = SSE | loopback;
    if ((REG32(base + SR) & (TFE | TNF)) != (TFE | TNF)) {
        return 1;
    }
    REG32(base + DR) = 0x5au;
    if ((REG32(base + SR) & RNE) == 0u) {
        return 2;
    }
    if (REG32(base + DR) != expected) {
        return 3;
    }
    REG32(base + IMSC) = TXIM;
    if ((REG32(base + MIS) & TXIM) == 0u) {
        return 4;
    }
    return 0;
}

int main(void)
{
    int result = check_spi(SPI0_BASE, LBM, 0x5au);
    if (result != 0) {
        return result;
    }
    return check_spi(SPI1_BASE, 0u, 0u);
}
