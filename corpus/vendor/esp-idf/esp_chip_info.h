#ifndef REMU_ESP_CHIP_INFO_H
#define REMU_ESP_CHIP_INFO_H
#include <stdint.h>
typedef struct {
    uint32_t features;
    uint8_t cores;
    uint16_t revision;
} esp_chip_info_t;
#define CHIP_FEATURE_WIFI_BGN (1u << 0)
#define CHIP_FEATURE_BT (1u << 1)
#define CHIP_FEATURE_BLE (1u << 2)
#define CHIP_FEATURE_IEEE802154 (1u << 3)
#define CHIP_FEATURE_EMB_FLASH (1u << 4)
void esp_chip_info(esp_chip_info_t *information);
#endif
