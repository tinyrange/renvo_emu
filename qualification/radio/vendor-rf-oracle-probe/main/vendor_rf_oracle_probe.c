/* SPDX-License-Identifier: Apache-2.0 */

/*
 * Public-API perturbation oracle for the genuine ESP32-C6 RF/PHY libraries.
 *
 * This image is intentionally vendor-backed: it is reverse-engineering
 * evidence, not the independent acceptance firmware.  UART checkpoints are
 * flushed around every public API operation so one ordered bus stream can
 * partition RF-page activity without hooks or PC-dependent behavior.
 */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "esp_err.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

enum {
    POWER_LOW_QDBM = 32,    /* 8 dBm requested maximum. */
    POWER_MEDIUM_QDBM = 56, /* 14 dBm requested maximum. */
    POWER_HIGH_QDBM = 80,   /* 20 dBm requested maximum. */
};

static uint8_t probe_request[] = {
    0x40, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00,
    0x00, 0x0f,
    'R', 'E', 'M', 'U', '-', 'R', 'F', '-', '0', '0', '-', 'P', '0', '0', '0',
    0x01, 0x01, 0x82,
};

static void checkpoint(const char *phase)
{
    printf("REMU_RF_ORACLE %s\n", phase);
    fflush(stdout);
}

static void initialize_wifi(void)
{
    wifi_init_config_t initialization = WIFI_INIT_CONFIG_DEFAULT();
    initialization.nvs_enable = 0;
    ESP_ERROR_CHECK(esp_wifi_init(&initialization));
    ESP_ERROR_CHECK(esp_wifi_set_storage(WIFI_STORAGE_RAM));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_start());
    ESP_ERROR_CHECK(esp_wifi_get_mac(WIFI_IF_STA, &probe_request[10]));
}

static void run_stage(const char *stage, uint8_t channel, int8_t power_qdbm)
{
    printf("REMU_RF_ORACLE BEGIN stage=%s channel=%u power_qdbm=%d\n",
           stage, (unsigned)channel, (int)power_qdbm);
    fflush(stdout);

    esp_err_t channel_result =
        esp_wifi_set_channel(channel, WIFI_SECOND_CHAN_NONE);
    esp_err_t power_result = esp_wifi_set_max_tx_power(power_qdbm);
    uint8_t observed_channel = 0;
    wifi_second_chan_t observed_second = WIFI_SECOND_CHAN_NONE;
    int8_t observed_power = 0;
    esp_err_t get_channel_result =
        esp_wifi_get_channel(&observed_channel, &observed_second);
    esp_err_t get_power_result = esp_wifi_get_max_tx_power(&observed_power);

    probe_request[34] = (uint8_t)('0' + channel / 10);
    probe_request[35] = (uint8_t)('0' + channel % 10);
    probe_request[38] = (uint8_t)('0' + (power_qdbm / 100) % 10);
    probe_request[39] = (uint8_t)('0' + (power_qdbm / 10) % 10);
    probe_request[40] = (uint8_t)('0' + power_qdbm % 10);
    esp_err_t tx_result = esp_wifi_80211_tx(
        WIFI_IF_STA, probe_request, sizeof(probe_request), true);

    printf("REMU_RF_ORACLE END stage=%s set_channel=%d set_power=%d "
           "get_channel=%d observed_channel=%u observed_second=%u "
           "get_power=%d observed_power_qdbm=%d tx=%d\n",
           stage, (int)channel_result, (int)power_result,
           (int)get_channel_result, (unsigned)observed_channel,
           (unsigned)observed_second, (int)get_power_result,
           (int)observed_power, (int)tx_result);
    fflush(stdout);
    vTaskDelay(pdMS_TO_TICKS(20));
}

void app_main(void)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());

    checkpoint("COLD_INIT_BEGIN");
    initialize_wifi();
    checkpoint("COLD_INIT_END");

    /* Isolate channel selection while holding the requested power constant. */
    run_stage("CHANNEL_1", 1, POWER_MEDIUM_QDBM);
    run_stage("CHANNEL_6", 6, POWER_MEDIUM_QDBM);
    run_stage("CHANNEL_11", 11, POWER_MEDIUM_QDBM);

    /* Isolate three monotonic power selections on one fixed channel. */
    run_stage("POWER_LOW", 6, POWER_LOW_QDBM);
    run_stage("POWER_MEDIUM", 6, POWER_MEDIUM_QDBM);
    run_stage("POWER_HIGH", 6, POWER_HIGH_QDBM);

    checkpoint("WARM_DISABLE_BEGIN");
    ESP_ERROR_CHECK(esp_wifi_stop());
    checkpoint("WARM_DISABLE_END");
    checkpoint("WARM_ENABLE_BEGIN");
    ESP_ERROR_CHECK(esp_wifi_start());
    checkpoint("WARM_ENABLE_END");
    run_stage("WARM_CHANNEL_6", 6, POWER_MEDIUM_QDBM);

    checkpoint("RADIO_RESET_BEGIN");
    ESP_ERROR_CHECK(esp_wifi_stop());
    ESP_ERROR_CHECK(esp_wifi_deinit());
    checkpoint("RADIO_RESET_DEINITIALIZED");
    initialize_wifi();
    checkpoint("RADIO_RESET_END");
    run_stage("RESET_CHANNEL_11", 11, POWER_MEDIUM_QDBM);

    checkpoint("DONE");
}
