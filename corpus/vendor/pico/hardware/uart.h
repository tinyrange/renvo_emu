#ifndef REMU_PICO_HARDWARE_UART_H
#define REMU_PICO_HARDWARE_UART_H

#include <stdint.h>

typedef unsigned int uint;
typedef struct uart_inst uart_inst_t;

extern uart_inst_t *const uart0;

void uart_init(uart_inst_t *uart, uint baudrate);
void uart_putc_raw(uart_inst_t *uart, char c);
void uart_putc(uart_inst_t *uart, char c);
void uart_puts(uart_inst_t *uart, const char *s);

#define UART_FUNCSEL_NUM(uart, pin) (GPIO_FUNC_UART)

#endif
