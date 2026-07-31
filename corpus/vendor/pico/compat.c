#include "pico/stdlib.h"

#define REG32(address) (*(volatile uint32_t *)(address))
static uint32_t sleeps;

void gpio_init(unsigned int pin) { (void)pin; }
void gpio_set_dir(unsigned int pin, bool output)
{
    REG32(output ? 0xd0000024u : 0xd0000028u) = 1u << pin;
}
void gpio_put(unsigned int pin, bool value)
{
    REG32(value ? 0xd0000014u : 0xd0000018u) = 1u << pin;
}
void sleep_ms(uint32_t milliseconds)
{
    (void)milliseconds;
    if (++sleeps == 3u) REG32(0xfffffff0u) = 0u;
}
