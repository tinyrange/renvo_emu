#include "pico/stdlib.h"
#include "hardware/pwm.h"
#include "hardware/uart.h"

#define REG32(address) (*(volatile uint32_t *)(address))
static uint32_t sleeps;

struct uart_inst {
    uintptr_t base;
};

static struct uart_inst uart0_state = {0x40034000u};
uart_inst_t *const uart0 = &uart0_state;

void gpio_init(unsigned int pin) { (void)pin; }
void gpio_set_dir(unsigned int pin, bool output)
{
    REG32(output ? 0xd0000024u : 0xd0000028u) = 1u << pin;
}
void gpio_put(unsigned int pin, bool value)
{
    REG32(value ? 0xd0000014u : 0xd0000018u) = 1u << pin;
}
void gpio_set_function(unsigned int pin, unsigned int function)
{
    REG32(0x40014000u + (pin * 8u)) = function;
}
void sleep_ms(uint32_t milliseconds)
{
    (void)milliseconds;
    if (++sleeps == 3u) REG32(0xfffffff0u) = 0u;
}

void uart_init(uart_inst_t *uart, uint baudrate)
{
    (void)uart;
    (void)baudrate;
}

void uart_putc_raw(uart_inst_t *uart, char c)
{
    REG32(uart->base) = (uint8_t)c;
}

void uart_putc(uart_inst_t *uart, char c)
{
    if (c == '\n') {
        uart_putc_raw(uart, '\r');
    }
    uart_putc_raw(uart, c);
}

void uart_puts(uart_inst_t *uart, const char *s)
{
    while (*s != '\0') {
        uart_putc(uart, *s++);
    }
}

uint pwm_gpio_to_slice_num(uint gpio)
{
    return gpio / 2u;
}

void pwm_set_wrap(uint slice_num, uint16_t wrap)
{
    REG32(0x40050000u + (slice_num * 0x14u) + 0x10u) = wrap;
}

void pwm_set_chan_level(uint slice_num, enum pwm_chan channel, uint16_t level)
{
    uintptr_t address = 0x40050000u + (slice_num * 0x14u) + 0x0cu;
    uint32_t current = REG32(address);
    if (channel == PWM_CHAN_A) {
        current = (current & 0xffff0000u) | level;
    } else {
        current = (current & 0x0000ffffu) | ((uint32_t)level << 16);
    }
    REG32(address) = current;
}

void pwm_set_enabled(uint slice_num, bool enabled)
{
    REG32(0x400500a0u) = enabled ? (1u << slice_num) : 0u;
}
