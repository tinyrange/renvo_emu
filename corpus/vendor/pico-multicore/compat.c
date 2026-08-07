#include <stdint.h>
#include <stddef.h>

#include "pico/multicore.h"
#include "pico/stdlib.h"

#define REG32(address) (*(volatile uint32_t *)(address))
#define UART0_DR 0x40034000u
#define SIO_FIFO_ST 0xd0000050u
#define SIO_FIFO_WR 0xd0000054u
#define SIO_FIFO_RD 0xd0000058u

static void uart_putc(char value)
{
    REG32(UART0_DR) = (uint8_t)value;
}

static void uart_puts(const char *value)
{
    while (*value != '\0') {
        uart_putc(*value++);
    }
}

int printf(const char *format, ...)
{
    uart_puts(format);
    return 0;
}

void stdio_init_all(void) {}

void tight_loop_contents(void) {}

void multicore_launch_core1(void (*entry)(void))
{
    const uint32_t sequence[] = {
        0,
        0,
        1,
        0x10000000u,
        0x20040000u,
        (uint32_t)(uintptr_t)entry,
    };
    for (size_t index = 0; index < sizeof(sequence) / sizeof(sequence[0]); ++index) {
        REG32(SIO_FIFO_WR) = sequence[index];
        /* The functional ROM echoes launch words to core 0. The SDK drains
         * those acknowledgements before waiting for the application FIFO. */
        (void)REG32(SIO_FIFO_RD);
    }
    /* The final entry word is echoed as the ROM transitions core 1 out of
     * reset, so consume that last acknowledgement as well. */
    (void)REG32(SIO_FIFO_RD);
}

void multicore_fifo_push_blocking(uint32_t value)
{
    REG32(SIO_FIFO_WR) = value;
}

uint32_t multicore_fifo_pop_blocking(void)
{
    while ((REG32(SIO_FIFO_ST) & 1u) == 0) {
        tight_loop_contents();
    }
    return REG32(SIO_FIFO_RD);
}
