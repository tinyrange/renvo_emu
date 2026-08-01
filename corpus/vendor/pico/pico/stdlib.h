#ifndef REMU_PICO_STDLIB_H
#define REMU_PICO_STDLIB_H

#include <stdbool.h>
#include <stdint.h>

#define PICO_DEFAULT_LED_PIN 25u
#define GPIO_OUT true
#define PICO_OK 0

void gpio_init(unsigned int pin);
void gpio_set_dir(unsigned int pin, bool output);
void gpio_put(unsigned int pin, bool value);
void sleep_ms(uint32_t milliseconds);

#endif
