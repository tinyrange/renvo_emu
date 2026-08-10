/*
 * Project-owned qualification firmware for genuine ESP-IDF radio libraries.
 *
 * This intentionally uses only the public ESP-IDF API. The emulator executes
 * the linked vendor ROM, PHY, Wi-Fi and RTOS code without symbol interception.
 */

#include <stdio.h>

#include "esp_err.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_wifi.h"

void app_main(void)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    ESP_ERROR_CHECK(esp_netif_create_default_wifi_sta() != NULL ? ESP_OK : ESP_FAIL);

    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    init.nvs_enable = 0;
    ESP_ERROR_CHECK(esp_wifi_init(&init));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_start());

    wifi_scan_config_t scan = {
        .channel = 1,
        .show_hidden = true,
        .scan_type = WIFI_SCAN_TYPE_ACTIVE,
        .scan_time = {
            .active = {
                .min = 5,
                .max = 10,
            },
        },
    };
    esp_err_t result = esp_wifi_scan_start(&scan, true);
    uint16_t count = 0;
    if (result == ESP_OK) {
        result = esp_wifi_scan_get_ap_num(&count);
    }
    printf("REMU_VENDOR_WIFI_SCAN_DONE result=%d count=%u\n",
           (int)result,
           (unsigned)count);
    fflush(stdout);
}
