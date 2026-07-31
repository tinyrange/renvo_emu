#ifndef RENVO_ESP_SYSTEM_H
#define RENVO_ESP_SYSTEM_H
#include <stdint.h>
uint32_t esp_get_minimum_free_heap_size(void);
void esp_restart(void) __attribute__((noreturn));
#endif
