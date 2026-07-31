#ifndef RENVO_ESP_FLASH_H
#define RENVO_ESP_FLASH_H
#include <stdint.h>
typedef int esp_err_t;
#define ESP_OK 0
esp_err_t esp_flash_get_size(void *chip, uint32_t *size);
#endif
