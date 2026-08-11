/* SPDX-License-Identifier: Apache-2.0 */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#include "esp_err.h"
#include "esp_zigbee.h"
#include "ezbee/core.h"
#include "ezbee/secur.h"
#include "ezbee/zha.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "nvs_flash.h"

#define QUALIFICATION_CHANNEL 11
#define QUALIFICATION_ENDPOINT 10
#define QUALIFICATION_STORAGE_PARTITION "zb_storage"
#define QUALIFICATION_PAN_ID 0xf67c

static const ezb_extaddr_t qualification_extended_address = {
    .u8 = {0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01},
};
static const ezb_extpanid_t qualification_extended_pan_id = {
    .u8 = {0x52, 0x45, 0x4d, 0x55, 0x2d, 0x5a, 0x42, 0x01},
};
static const uint8_t qualification_network_key[16] = {
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
};

static bool handle_app_signal(const ezb_app_signal_t *app_signal)
{
    ezb_app_signal_type_t signal_type = ezb_app_signal_get_type(app_signal);

    printf("REMU_VENDOR_ZIGBEE_SIGNAL type=%u name=%s\n", (unsigned)signal_type,
           ezb_app_signal_to_string(signal_type));
    switch (signal_type) {
    case EZB_ZDO_SIGNAL_SKIP_STARTUP: {
        esp_err_t result =
            ezb_bdb_start_top_level_commissioning(EZB_BDB_MODE_INITIALIZATION);
        printf("REMU_VENDOR_ZIGBEE_INITIALIZE result=%d\n", (int)result);
        break;
    }
    case EZB_BDB_SIGNAL_DEVICE_FIRST_START:
    case EZB_BDB_SIGNAL_DEVICE_REBOOT: {
        ezb_bdb_comm_status_t status =
            *(const ezb_bdb_comm_status_t *)ezb_app_signal_get_params(app_signal);
        bool factory_new = ezb_bdb_is_factory_new();
        printf("REMU_VENDOR_ZIGBEE_START status=%u factory_new=%u\n",
               (unsigned)status, factory_new ? 1u : 0u);
        if (status == EZB_BDB_STATUS_SUCCESS && factory_new) {
            esp_err_t result = ezb_bdb_start_top_level_commissioning(
                EZB_BDB_MODE_NETWORK_FORMATION);
            printf("REMU_VENDOR_ZIGBEE_FORMATION_START result=%d\n",
                   (int)result);
        }
        break;
    }
    case EZB_BDB_SIGNAL_FORMATION: {
        ezb_bdb_comm_status_t status =
            *(const ezb_bdb_comm_status_t *)ezb_app_signal_get_params(app_signal);
        printf("REMU_VENDOR_ZIGBEE_FORMATION_DONE status=%u pan=%u channel=%u short=%u\n",
               (unsigned)status, (unsigned)ezb_nwk_get_panid(),
               (unsigned)ezb_nwk_get_current_channel(),
               (unsigned)ezb_nwk_get_short_address());
        if (status == EZB_BDB_STATUS_SUCCESS) {
            esp_err_t result = ezb_bdb_start_top_level_commissioning(
                EZB_BDB_MODE_NETWORK_STEERING);
            printf("REMU_VENDOR_ZIGBEE_STEERING_START result=%d\n",
                   (int)result);
        }
        break;
    }
    case EZB_BDB_SIGNAL_STEERING: {
        ezb_bdb_comm_status_t status =
            *(const ezb_bdb_comm_status_t *)ezb_app_signal_get_params(app_signal);
        printf("REMU_VENDOR_ZIGBEE_STEERING_DONE status=%u\n",
               (unsigned)status);
        break;
    }
    default:
        break;
    }
    return true;
}

static esp_err_t register_qualification_endpoint(void)
{
    ezb_af_device_desc_t device = ezb_af_create_device_desc();
    ezb_zha_on_off_light_config_t config = EZB_ZHA_ON_OFF_LIGHT_CONFIG();
    ezb_af_ep_desc_t endpoint =
        ezb_zha_create_on_off_light(QUALIFICATION_ENDPOINT, &config);

    return ezb_af_device_add_endpoint_desc(device, endpoint) == ESP_OK
               ? ezb_af_device_desc_register(device)
               : ESP_FAIL;
}

static void zigbee_task(void *argument)
{
    (void)argument;
    esp_zigbee_config_t config = {
        .device_config = {
            .device_type = EZB_NWK_DEVICE_TYPE_COORDINATOR,
            .install_code_policy = false,
            .zczr_config = {.max_children = 4},
        },
        .platform_config = {
            .storage_partition_name = QUALIFICATION_STORAGE_PARTITION,
            .radio_config = {.radio_mode = ESP_ZIGBEE_RADIO_MODE_NATIVE},
        },
    };

    esp_err_t result = esp_zigbee_init(&config);
    printf("REMU_VENDOR_ZIGBEE_INIT result=%d\n", (int)result);
    if (result == ESP_OK) {
        ezb_set_extended_address(&qualification_extended_address);
        ezb_set_use_extended_panid(&qualification_extended_pan_id);
        ezb_set_panid(QUALIFICATION_PAN_ID);
        result = ezb_secur_set_network_key(qualification_network_key);
        printf("REMU_VENDOR_ZIGBEE_IDENTITY result=%d pan=%u\n", (int)result,
               QUALIFICATION_PAN_ID);
    }
    if (result == ESP_OK) {
        ezb_aps_secur_enable_distributed_security(false);
        result = ezb_bdb_set_primary_channel_set(1u << QUALIFICATION_CHANNEL);
    }
    if (result == ESP_OK) {
        result = ezb_bdb_set_secondary_channel_set(0);
    }
    if (result == ESP_OK) {
        result = ezb_app_signal_add_handler(handle_app_signal);
    }
    if (result == ESP_OK) {
        result = register_qualification_endpoint();
    }
    printf("REMU_VENDOR_ZIGBEE_CONFIG result=%d channel=%u endpoint=%u\n",
           (int)result, QUALIFICATION_CHANNEL, QUALIFICATION_ENDPOINT);
    if (result == ESP_OK) {
        result = esp_zigbee_start(false);
    }
    printf("REMU_VENDOR_ZIGBEE_STACK_START result=%d\n", (int)result);
    if (result == ESP_OK) {
        esp_zigbee_launch_mainloop();
    }
    esp_zigbee_deinit();
    vTaskDelete(NULL);
}

void app_main(void)
{
    esp_err_t result = nvs_flash_init();
    if (result == ESP_ERR_NVS_NO_FREE_PAGES ||
        result == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        result = nvs_flash_erase();
        if (result == ESP_OK) {
            result = nvs_flash_init();
        }
    }
    if (result == ESP_OK) {
        result = nvs_flash_init_partition(QUALIFICATION_STORAGE_PARTITION);
    }
    printf("REMU_VENDOR_ZIGBEE_PLATFORM result=%d\n", (int)result);
    if (result == ESP_OK) {
        BaseType_t created = xTaskCreate(zigbee_task, "zigbee", 6144, NULL, 5,
                                         NULL);
        printf("REMU_VENDOR_ZIGBEE_TASK created=%u\n",
               created == pdPASS ? 1u : 0u);
    }
}
