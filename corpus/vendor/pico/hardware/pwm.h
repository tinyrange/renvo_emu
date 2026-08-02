#ifndef REMU_PICO_HARDWARE_PWM_H
#define REMU_PICO_HARDWARE_PWM_H

#include <stdbool.h>
#include <stdint.h>

typedef unsigned int uint;

enum pwm_chan {
    PWM_CHAN_A = 0,
    PWM_CHAN_B = 1,
};

uint pwm_gpio_to_slice_num(uint gpio);
void pwm_set_wrap(uint slice_num, uint16_t wrap);
void pwm_set_chan_level(uint slice_num, enum pwm_chan channel, uint16_t level);
void pwm_set_enabled(uint slice_num, bool enabled);

#endif
