#ifndef REMU_HARMONY_DEFINITIONS_H
#define REMU_HARMONY_DEFINITIONS_H

#include <stdint.h>

void SYS_Initialize(void *data);
uint32_t remu_switch_get(void);
void remu_led_clear(void);
void remu_led_set(void);

#define SWITCH_Get() remu_switch_get()
#define LED_Clear() remu_led_clear()
#define LED_Set() remu_led_set()

#endif
