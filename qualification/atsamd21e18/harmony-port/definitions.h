#ifndef RENVO_HARMONY_DEFINITIONS_H
#define RENVO_HARMONY_DEFINITIONS_H

#include <stdint.h>

void SYS_Initialize(void *data);
uint32_t renvo_switch_get(void);
void renvo_led_clear(void);
void renvo_led_set(void);

#define SWITCH_Get() renvo_switch_get()
#define LED_Clear() renvo_led_clear()
#define LED_Set() renvo_led_set()

#endif
