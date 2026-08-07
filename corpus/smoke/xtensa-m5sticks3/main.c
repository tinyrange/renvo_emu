#include <stdint.h>

#define MMIO32(address) (*(volatile uint32_t *)(uintptr_t)(address))

#define GPIO_BASE 0x60004000u
#define GPIO_OUT 0x04u
#define GPIO_OUT1 0x10u
#define GPIO_ENABLE 0x20u
#define GPIO_ENABLE1 0x2cu
#define GPIO_IN 0x3cu
#define GPIO_IN1 0x40u

#define I2C1_BASE 0x60027000u
#define I2C_CTR 0x04u
#define I2C_DATA 0x1cu
#define I2C_COMMAND0 0x58u

#define SPI3_BASE 0x60025000u
#define SPI_CMD 0x00u
#define SPI_USER 0x10u
#define SPI_MS_DLEN 0x1cu
#define SPI_W0 0x98u

#define I2S0_BASE 0x6000f000u
#define I2S1_BASE 0x6002d000u
#define I2S_RX_CONF 0x20u
#define I2S_TX_CONF 0x24u
#define I2S_SINGLE_DATA 0x68u

#define RMT_BASE 0x60016000u
#define RMT_CH0_DATA 0x00u
#define RMT_CH0_CONF0 0x20u
#define RMT_CH4_DATA 0x10u
#define RMT_CH4_CONF1 0x34u
#define RMT_INT_RAW 0x70u

#define COMMAND(bytes, opcode) ((uint32_t)(bytes) | ((uint32_t)(opcode) << 11))

static uint32_t gpio_high;

static void gpio_high_write(uint32_t pin, uint32_t high)
{
    const uint32_t bit = 1u << (pin - 32u);
    if (high != 0u) {
        gpio_high |= bit;
    } else {
        gpio_high &= ~bit;
    }
    MMIO32(GPIO_BASE + GPIO_OUT1) = gpio_high;
}

static void i2c1_write(uint8_t address, uint8_t reg, uint8_t value)
{
    MMIO32(I2C1_BASE + I2C_DATA) = (uint32_t)(address << 1);
    MMIO32(I2C1_BASE + I2C_DATA) = reg;
    MMIO32(I2C1_BASE + I2C_DATA) = value;
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 0u) = COMMAND(0, 0);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 4u) = COMMAND(3, 1);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 8u) = COMMAND(0, 3);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 12u) = COMMAND(0, 4);
    MMIO32(I2C1_BASE + I2C_CTR) = 0x30u;
}

static uint8_t i2c1_read8(uint8_t address, uint8_t reg)
{
    MMIO32(I2C1_BASE + I2C_DATA) = (uint32_t)(address << 1);
    MMIO32(I2C1_BASE + I2C_DATA) = reg;
    MMIO32(I2C1_BASE + I2C_DATA) = (uint32_t)((address << 1) | 1u);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 0u) = COMMAND(0, 0);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 4u) = COMMAND(2, 1);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 8u) = COMMAND(0, 0);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 12u) = COMMAND(1, 1);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 16u) = COMMAND(1, 2);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 20u) = COMMAND(0, 3);
    MMIO32(I2C1_BASE + I2C_COMMAND0 + 24u) = COMMAND(0, 4);
    MMIO32(I2C1_BASE + I2C_CTR) = 0x30u;
    return (uint8_t)MMIO32(I2C1_BASE + I2C_DATA);
}

static uint32_t rmt_item(uint32_t duration0, uint32_t level0,
                         uint32_t duration1, uint32_t level1)
{
    return duration0 | (level0 << 15) | (duration1 << 16) | (level1 << 31);
}

static uint32_t pack_word(const uint8_t *bytes, uint32_t length)
{
    uint32_t word = 0;
    for (uint32_t index = 0; index < length; ++index) {
        word |= (uint32_t)bytes[index] << (24u - index * 8u);
    }
    return word;
}

static void spi3_write(const uint8_t *bytes, uint32_t length, uint32_t data_phase)
{
    gpio_high_write(45u, data_phase);
    gpio_high_write(41u, 0u);
    MMIO32(SPI3_BASE + SPI_W0) = pack_word(bytes, length);
    MMIO32(SPI3_BASE + SPI_MS_DLEN) = length * 8u - 1u;
    MMIO32(SPI3_BASE + SPI_USER) = 1u << 27;
    MMIO32(SPI3_BASE + SPI_CMD) = 1u << 24;
    gpio_high_write(41u, 1u);
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;
    const uint32_t low_outputs = 1u << 21;
    const uint32_t high_outputs = (1u << (38u - 32u)) |
                                  (1u << (41u - 32u)) |
                                  (1u << (45u - 32u));
    const uint8_t invert_on[] = {0x21};
    const uint8_t display_on[] = {0x29};
    const uint8_t column_command[] = {0x2a};
    const uint8_t column_data[] = {0x00, 0x34, 0x00, 0xba};
    const uint8_t row_command[] = {0x2b};
    const uint8_t row_data[] = {0x00, 0x28, 0x01, 0x17};
    const uint8_t write_command[] = {0x2c};
    const uint8_t pixels[] = {0xf8, 0x00, 0x07, 0xe0};

    MMIO32(GPIO_BASE + GPIO_OUT) = low_outputs;
    MMIO32(GPIO_BASE + GPIO_ENABLE) = low_outputs;
    gpio_high = high_outputs;
    MMIO32(GPIO_BASE + GPIO_OUT1) = gpio_high;
    MMIO32(GPIO_BASE + GPIO_ENABLE1) = high_outputs;

    if ((MMIO32(GPIO_BASE + GPIO_IN) & (1u << 11)) != 0u) {
        failure = 1;
    }
    if ((MMIO32(GPIO_BASE + GPIO_IN) & (1u << 12)) == 0u) {
        failure = 2;
    }
    if ((MMIO32(GPIO_BASE + GPIO_IN1) & (1u << (42u - 32u))) == 0u) {
        failure = 3;
    }

    i2c1_write(0x6eu, 0x06u, 0x0fu);
    i2c1_write(0x6eu, 0x10u, 0x0cu);
    i2c1_write(0x6eu, 0x11u, 0x0cu);
    if (i2c1_read8(0x6eu, 0x00u) != 0x01u) {
        failure = 4;
    }

    if (i2c1_read8(0x68u, 0x00u) != 0x24u) {
        failure = 5;
    }
    i2c1_write(0x68u, 0x5eu, 0xa5u);
    i2c1_write(0x68u, 0x59u, 0x01u);
    i2c1_write(0x68u, 0x7du, 0x06u);
    if (i2c1_read8(0x68u, 0x0cu) != 100u) {
        failure = 6;
    }

    i2c1_write(0x18u, 0x00u, 0x80u);
    i2c1_write(0x18u, 0x0du, 0x01u);
    i2c1_write(0x18u, 0x0eu, 0x02u);
    i2c1_write(0x18u, 0x12u, 0x00u);
    i2c1_write(0x18u, 0x13u, 0x10u);
    i2c1_write(0x18u, 0x17u, 0xffu);
    i2c1_write(0x18u, 0x32u, 0xbfu);

    MMIO32(I2S0_BASE + I2S_SINGLE_DATA) = 0xa55a1234u;
    MMIO32(I2S0_BASE + I2S_TX_CONF) = 1u << 2;
    MMIO32(I2S1_BASE + I2S_RX_CONF) = 1u << 2;
    if (MMIO32(I2S1_BASE + I2S_SINGLE_DATA) != 0x12345678u) {
        failure = 7;
    }

    MMIO32(RMT_BASE + RMT_CH0_DATA) = rmt_item(9000u, 1u, 4500u, 0u);
    MMIO32(RMT_BASE + RMT_CH0_DATA) = rmt_item(560u, 1u, 560u, 0u);
    MMIO32(RMT_BASE + RMT_CH0_CONF0) = 1u;
    MMIO32(RMT_BASE + RMT_CH4_CONF1) = 3u;
    MMIO32(RMT_BASE + RMT_CH4_CONF1) = 1u;
    if ((MMIO32(RMT_BASE + RMT_INT_RAW) & (1u << 16)) == 0u ||
        MMIO32(RMT_BASE + RMT_CH4_DATA) != rmt_item(9000u, 1u, 4500u, 0u)) {
        failure = 8;
    }

    spi3_write(invert_on, sizeof(invert_on), 0u);
    spi3_write(display_on, sizeof(display_on), 0u);
    spi3_write(column_command, sizeof(column_command), 0u);
    spi3_write(column_data, sizeof(column_data), 1u);
    spi3_write(row_command, sizeof(row_command), 0u);
    spi3_write(row_data, sizeof(row_data), 1u);
    spi3_write(write_command, sizeof(write_command), 0u);
    spi3_write(pixels, sizeof(pixels), 1u);

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
