#define REG32(address) (*(volatile unsigned int *)(address))

#define I2C_BASE 0x40090000u
#define I2C1_BASE 0x40098000u
#define I2C_TAR REG32(I2C_BASE + 0x04u)
#define I2C_DATA_CMD REG32(I2C_BASE + 0x10u)
#define I2C_ENABLE REG32(I2C_BASE + 0x6cu)
#define I2C_STATUS REG32(I2C_BASE + 0x70u)
#define I2C1_TAR REG32(I2C1_BASE + 0x04u)
#define I2C1_DATA_CMD REG32(I2C1_BASE + 0x10u)
#define I2C1_ENABLE REG32(I2C1_BASE + 0x6cu)

int main(void)
{
    I2C_TAR = 0x48u;
    I2C_ENABLE = 1u;
    I2C_DATA_CMD = 0x10u;
    I2C_DATA_CMD = (1u << 8) | (1u << 9);
    if ((I2C_STATUS & (1u << 3)) == 0u) {
        return 1;
    }
    if (I2C_DATA_CMD != 0xffu) {
        return 1;
    }
    I2C1_TAR = 0x48u;
    I2C1_ENABLE = 1u;
    I2C1_DATA_CMD = (1u << 8) | (1u << 9);
    return I2C1_DATA_CMD == 0xffu ? 0 : 1;
}
