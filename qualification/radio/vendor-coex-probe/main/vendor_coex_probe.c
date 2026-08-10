/*
 * Project-owned qualification firmware for genuine Wi-Fi/BLE coexistence.
 *
 * Only public ESP-IDF and NimBLE APIs are used. Renvo executes the linked
 * controller, ROM, PHY, scheduler, coexistence, and RTOS code without symbol
 * or address interception.
 */

#include <stdio.h>

#include "esp_err.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "host/ble_gap.h"
#include "host/ble_hs.h"
#include "host/ble_hs_adv.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"

static volatile bool wifi_started;
static volatile bool ble_advertising_started;
static volatile bool ble_scan_reported;

static void host_task(void *argument)
{
    (void)argument;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

static void on_wifi_event(void *argument, esp_event_base_t event_base,
                          int32_t event_id, void *event_data)
{
    (void)argument;
    (void)event_base;
    (void)event_data;
    if (event_id == WIFI_EVENT_AP_START) {
        wifi_started = true;
        printf("REMU_VENDOR_COEX_WIFI_STARTED\n");
        fflush(stdout);
    }
}

static int on_gap_event(struct ble_gap_event *event, void *argument)
{
    (void)argument;
    if (event->type == BLE_GAP_EVENT_DISC) {
        const struct ble_gap_disc_desc *report = &event->disc;
        ble_scan_reported = true;
        printf("REMU_VENDOR_COEX_BLE_SCAN_REPORT type=%u length=%u rssi=%d",
               report->event_type, report->length_data, report->rssi);
        for (uint8_t index = 0; index < report->length_data; ++index) {
            printf(" %02x", report->data[index]);
        }
        printf("\n");
        fflush(stdout);
    }
    return 0;
}

static void on_ble_sync(void)
{
    static const uint8_t random_static_address[6] = {
        0x16, 0x15, 0x14, 0x13, 0x12, 0xc1,
    };
    static const uint8_t advertisement[] = {
        0x02, BLE_HS_ADV_TYPE_FLAGS,
        BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP,
        0x0c, BLE_HS_ADV_TYPE_COMP_NAME,
        'R', 'e', 'n', 'v', 'o', '-', 'C', 'o', 'e', 'x', '1',
    };
    struct ble_gap_adv_params advertising = {0};
    struct ble_gap_disc_params discovery = {0};

    int result = ble_hs_id_set_rnd(random_static_address);
    if (result == 0) {
        result = ble_gap_adv_set_data(advertisement, sizeof(advertisement));
    }
    advertising.conn_mode = BLE_GAP_CONN_MODE_NON;
    advertising.disc_mode = BLE_GAP_DISC_MODE_GEN;
    if (result == 0) {
        result = ble_gap_adv_start(BLE_OWN_ADDR_RANDOM, NULL, BLE_HS_FOREVER,
                                   &advertising, NULL, NULL);
    }
    ble_advertising_started = result == 0;
    printf("REMU_VENDOR_COEX_BLE_ADV_START result=%d wifi=%u\n", result,
           wifi_started ? 1u : 0u);
    fflush(stdout);

    vTaskDelay(pdMS_TO_TICKS(40));
    int stop_result = ble_gap_adv_stop();
    printf("REMU_VENDOR_COEX_BLE_ADV_STOP result=%d\n", stop_result);
    fflush(stdout);

    discovery.itvl = 16;
    discovery.window = 16;
    discovery.passive = 1;
    discovery.filter_duplicates = 0;
    int scan_result = ble_gap_disc(BLE_OWN_ADDR_RANDOM, BLE_HS_FOREVER,
                                   &discovery, on_gap_event, NULL);
    printf("REMU_VENDOR_COEX_BLE_SCAN_START result=%d\n", scan_result);
    fflush(stdout);
}

void app_main(void)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    ESP_ERROR_CHECK(esp_netif_create_default_wifi_ap() != NULL ? ESP_OK : ESP_FAIL);
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID,
                                               on_wifi_event, NULL));

    wifi_init_config_t initialization = WIFI_INIT_CONFIG_DEFAULT();
    initialization.nvs_enable = 0;
    ESP_ERROR_CHECK(esp_wifi_init(&initialization));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_AP));
    wifi_config_t configuration = {
        .ap = {
            .ssid = "REMU-COEX",
            .ssid_len = 9,
            .channel = 1,
            .authmode = WIFI_AUTH_OPEN,
            .max_connection = 2,
            .beacon_interval = 100,
        },
    };
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &configuration));
    ESP_ERROR_CHECK(esp_wifi_start());

    int ble_result = nimble_port_init();
    printf("REMU_VENDOR_COEX_BLE_INIT result=%d\n", ble_result);
    fflush(stdout);
    if (ble_result != 0) {
        return;
    }
    ble_hs_cfg.sync_cb = on_ble_sync;
    nimble_port_freertos_init(host_task);

    for (unsigned attempt = 0;
         attempt < 500 && (!wifi_started || !ble_advertising_started ||
                           !ble_scan_reported);
         ++attempt) {
        vTaskDelay(pdMS_TO_TICKS(10));
    }
    printf("REMU_VENDOR_COEX_DONE wifi=%u ble_adv=%u ble_scan=%u\n",
           wifi_started ? 1u : 0u,
           ble_advertising_started ? 1u : 0u,
           ble_scan_reported ? 1u : 0u);
    fflush(stdout);
}
