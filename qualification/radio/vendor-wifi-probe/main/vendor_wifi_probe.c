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
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static volatile bool station_connected;
static volatile int station_disconnect_reason = -1;

static void on_wifi_event(void *argument, esp_event_base_t event_base,
                          int32_t event_id, void *event_data)
{
    (void)argument;
    (void)event_base;
    if (event_id == WIFI_EVENT_STA_CONNECTED) {
        station_connected = true;
    } else if (event_id == WIFI_EVENT_STA_DISCONNECTED) {
        const wifi_event_sta_disconnected_t *event = event_data;
        station_disconnect_reason = event->reason;
    }
}

void app_main(void)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    ESP_ERROR_CHECK(esp_netif_create_default_wifi_sta() != NULL ? ESP_OK : ESP_FAIL);
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID,
                                               on_wifi_event, NULL));

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

    wifi_config_t configuration = {
        .sta = {
            .ssid = "REMU-AP",
            .bssid = {0x02, 0x52, 0x45, 0x4d, 0x55, 0x01},
            .bssid_set = true,
            .channel = 1,
            .threshold = {
                .authmode = WIFI_AUTH_OPEN,
            },
        },
    };
    result = esp_wifi_set_config(WIFI_IF_STA, &configuration);
    if (result == ESP_OK) {
        result = esp_wifi_connect();
    }
    printf("REMU_VENDOR_WIFI_CONNECT_START result=%d\n", (int)result);
    fflush(stdout);

    for (unsigned attempt = 0;
         result == ESP_OK && !station_connected && attempt < 200;
         ++attempt) {
        vTaskDelay(pdMS_TO_TICKS(10));
    }
    printf("REMU_VENDOR_WIFI_CONNECT_DONE connected=%u reason=%d\n",
           station_connected ? 1u : 0u, station_disconnect_reason);
    fflush(stdout);
}
