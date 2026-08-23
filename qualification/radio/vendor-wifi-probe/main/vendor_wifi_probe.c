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
#if CONFIG_SOC_WIFI_HE_SUPPORT
#include "esp_wifi_he.h"
#endif
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static volatile bool station_connected;
static volatile int station_disconnect_reason = -1;
#if CONFIG_SOC_WIFI_HE_SUPPORT
static volatile bool itwt_setup_reported;
static volatile esp_err_t itwt_setup_status = ESP_FAIL;
static volatile unsigned itwt_wakeup_count;
#endif

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
#if CONFIG_SOC_WIFI_HE_SUPPORT
    } else if (event_id == WIFI_EVENT_ITWT_SETUP) {
        const wifi_event_sta_itwt_setup_t *event = event_data;
        itwt_setup_status = event->status;
        itwt_setup_reported = true;
        printf("REMU_VENDOR_WIFI_ITWT_SETUP status=%d flow=%u twt=%u target=%llu\n",
               (int)event->status, (unsigned)event->config.flow_id,
               (unsigned)event->config.twt_id,
               (unsigned long long)event->target_wake_time);
        fflush(stdout);
    } else if (event_id == WIFI_EVENT_TWT_WAKEUP) {
        const wifi_event_sta_twt_wakeup_t *event = event_data;
        ++itwt_wakeup_count;
        printf("REMU_VENDOR_WIFI_ITWT_WAKEUP count=%u type=%u id=%u\n",
               itwt_wakeup_count, (unsigned)event->twt_type,
               (unsigned)event->flow_id);
        fflush(stdout);
#endif
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

#if CONFIG_SOC_WIFI_HE_SUPPORT
    if (station_connected) {
        wifi_phy_mode_t phy_mode = WIFI_PHY_MODE_LR;
        esp_err_t phy_result = esp_wifi_sta_get_negotiated_phymode(&phy_mode);
        printf("REMU_VENDOR_WIFI_PHY_MODE result=%d mode=%u\n",
               (int)phy_result, (unsigned)phy_mode);
        fflush(stdout);

        wifi_twt_config_t twt_config = {
            .post_wakeup_event = true,
            .twt_enable_keep_alive = false,
        };
        esp_err_t config_result = esp_wifi_sta_twt_config(&twt_config);
        esp_err_t offset_result =
            esp_wifi_sta_itwt_set_target_wake_time_offset(10000);
        wifi_itwt_setup_config_t setup = {
            .setup_cmd = TWT_SUGGEST,
            .trigger = 0,
            .flow_type = 0,
            .flow_id = 0,
            .wake_invl_expn = 5,
            .wake_duration_unit = 0,
            .min_wake_dura = 8,
            .wake_invl_mant = 1000,
            .twt_id = 7,
            .timeout_time_ms = 1000,
        };
        esp_err_t setup_result = esp_wifi_sta_itwt_setup(&setup);
        printf("REMU_VENDOR_WIFI_ITWT_START config=%d offset=%d setup=%d flow=%u\n",
               (int)config_result, (int)offset_result, (int)setup_result,
               (unsigned)setup.flow_id);
        fflush(stdout);

        for (unsigned attempt = 0;
             setup_result == ESP_OK && !itwt_setup_reported && attempt < 200;
             ++attempt) {
            vTaskDelay(pdMS_TO_TICKS(10));
        }
        int flow_bitmap = 0;
        esp_err_t bitmap_result =
            esp_wifi_sta_itwt_get_flow_id_status(&flow_bitmap);
        printf("REMU_VENDOR_WIFI_ITWT_STATUS reported=%u status=%d bitmap_result=%d bitmap=0x%x wakes=%u\n",
               itwt_setup_reported ? 1u : 0u, (int)itwt_setup_status,
               (int)bitmap_result, flow_bitmap, itwt_wakeup_count);
        fflush(stdout);
    }
#endif
}
