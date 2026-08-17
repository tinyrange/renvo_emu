/* SPDX-License-Identifier: Apache-2.0 */
#include "mmio.h"
#include "registers.h"
#include "uart.h"

#define UART_RXFIFO_COUNT_MASK 0x3ffu

void c6_uart_putc(char value)
{
    c6_write32(C6_REG_UART_FIFO, (uint8_t)value);
}

void c6_uart_puts(const char *value)
{
    while (*value != '\0') {
        c6_uart_putc(*value++);
    }
}

void c6_uart_put_u32(uint32_t value)
{
    char digits[10];
    uint32_t count = 0;
    do {
        digits[count++] = (char)('0' + value % 10u);
        value /= 10u;
    } while (value != 0);
    while (count != 0) {
        c6_uart_putc(digits[--count]);
    }
}

int c6_uart_getc_nonblocking(void)
{
    if ((c6_read32(C6_REG_UART_STATUS) & UART_RXFIFO_COUNT_MASK) == 0) {
        return C6_UART_NO_DATA;
    }
    return (int)(c6_read32(C6_REG_UART_FIFO) & 0xffu);
}
