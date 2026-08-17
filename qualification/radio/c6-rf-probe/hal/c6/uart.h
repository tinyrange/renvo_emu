/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_UART_H
#define REMU_C6_UART_H

#include <stdint.h>

enum c6_uart_result {
    C6_UART_NO_DATA = -1,
};

void c6_uart_putc(char value);
void c6_uart_puts(const char *value);
void c6_uart_put_u32(uint32_t value);
int c6_uart_getc_nonblocking(void);

#endif
