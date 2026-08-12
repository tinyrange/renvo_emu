/* SPDX-License-Identifier: Apache-2.0 */

#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include "esp_err.h"
#include "esp_event.h"
#include "esp_openthread.h"
#include "esp_openthread_lock.h"
#include "esp_openthread_types.h"
#include "esp_vfs_eventfd.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "nvs_flash.h"
#include "nvs.h"
#include "openthread/cli.h"
#include "openthread/tasklet.h"
#include "openthread/thread.h"
#include "openthread/thread_ftd.h"
#include "psa/crypto.h"

static void report_psa_key_storage(void)
{
    static const uint8_t key[16] = {
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    };
    psa_status_t init_status = psa_crypto_init();
    psa_key_attributes_t attributes = PSA_KEY_ATTRIBUTES_INIT;
    psa_key_id_t key_id = 0x21000;
    psa_set_key_id(&attributes, key_id);
    psa_set_key_lifetime(&attributes, PSA_KEY_LIFETIME_PERSISTENT);
    psa_set_key_type(&attributes, PSA_KEY_TYPE_HMAC);
    psa_set_key_algorithm(&attributes, PSA_ALG_HMAC(PSA_ALG_SHA_256));
    psa_set_key_usage_flags(&attributes,
                            PSA_KEY_USAGE_SIGN_HASH | PSA_KEY_USAGE_EXPORT);
    (void)psa_destroy_key(key_id);
    psa_status_t import_status =
        psa_import_key(&attributes, key, sizeof(key), &key_id);
    printf("REMU_VENDOR_THREAD_PSA init=%ld import=%ld key=%lu\n",
           (long)init_status, (long)import_status, (unsigned long)key_id);
    if (import_status == PSA_SUCCESS) {
        (void)psa_destroy_key(key_id);
    }
    psa_reset_key_attributes(&attributes);
}

static void report_nvs_round_trip(void)
{
    nvs_handle_t handle = 0;
    uint32_t written = 0x72656d75;
    uint32_t read = 0;
    esp_err_t open_status = nvs_open("remu_diag", NVS_READWRITE, &handle);
    esp_err_t set_status = ESP_FAIL;
    esp_err_t commit_status = ESP_FAIL;
    esp_err_t get_status = ESP_FAIL;
    if (open_status == ESP_OK) {
        set_status = nvs_set_u32(handle, "roundtrip", written);
        if (set_status == ESP_OK) {
            commit_status = nvs_commit(handle);
        }
        if (commit_status == ESP_OK) {
            get_status = nvs_get_u32(handle, "roundtrip", &read);
        }
        nvs_close(handle);
    }
    printf("REMU_VENDOR_THREAD_NVS open=%d set=%d commit=%d get=%d "
           "value=%08lx\n",
           (int)open_status, (int)set_status, (int)commit_status,
           (int)get_status, (unsigned long)read);
}

static int cli_output(void *context, const char *format, va_list arguments)
{
    (void)context;
    char output[256];
    int length = vsnprintf(output, sizeof(output), format, arguments);
    if (length > 0) {
        printf("REMU_VENDOR_THREAD_CLI_OUTPUT %s", output);
    }
    return length;
}

static void run_cli(const char *command)
{
    char line[192];
    snprintf(line, sizeof(line), "%s", command);
    esp_openthread_lock_acquire(portMAX_DELAY);
    otCliInputLine(line);
    esp_openthread_lock_release();
    printf("REMU_VENDOR_THREAD_CLI_COMMAND %s\n", command);
}

static void start_as_leader(otInstance *instance)
{
    char start[] = "thread start";
    char state[] = "state";
    char dataset[] = "dataset active -x";
    char ipaddr[] = "ipaddr";
    char ping[] = "ping ff02::1 16 1";
    esp_openthread_lock_acquire(portMAX_DELAY);
    otCliInputLine(start);
    otError leader_error = otThreadBecomeLeader(instance);
    otDeviceRole role = otThreadGetDeviceRole(instance);
    otCliInputLine(state);
    otCliInputLine(dataset);
    otCliInputLine(ipaddr);
    otCliInputLine(ping);
    unsigned tasklet_passes = 0;
    while (otTaskletsArePending(instance) && tasklet_passes < 64) {
        otTaskletsProcess(instance);
        tasklet_passes++;
    }
    printf("REMU_VENDOR_THREAD_CLI_COMMAND thread start\n");
    printf("REMU_VENDOR_THREAD_CLI_COMMAND ping ff02::1 16 1\n");
    printf("REMU_VENDOR_THREAD_BECOME_LEADER error=%u role=%u leader=%u\n",
           (unsigned)leader_error, (unsigned)role,
           role == OT_DEVICE_ROLE_LEADER ? 1u : 0u);
    printf("REMU_VENDOR_THREAD_TASKLETS passes=%u pending=%u\n",
           tasklet_passes, otTaskletsArePending(instance) ? 1u : 0u);
    esp_openthread_lock_release();
}

void app_main(void)
{
    esp_err_t result = nvs_flash_erase();
    if (result == ESP_OK) {
        result = nvs_flash_init();
    }
    if (result == ESP_ERR_NVS_NO_FREE_PAGES ||
        result == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        result = nvs_flash_erase();
        if (result == ESP_OK) {
            result = nvs_flash_init();
        }
    }
    if (result == ESP_OK) {
        result = esp_event_loop_create_default();
    }
    if (result == ESP_OK) {
        esp_vfs_eventfd_config_t eventfd_config = {.max_fds = 3};
        result = esp_vfs_eventfd_register(&eventfd_config);
    }
    printf("REMU_VENDOR_THREAD_PLATFORM result=%d\n", (int)result);
    if (result != ESP_OK) {
        return;
    }
    report_nvs_round_trip();
    report_psa_key_storage();

    static esp_openthread_config_t config = {
        .platform_config = {
            .radio_config = {.radio_mode = RADIO_MODE_NATIVE},
            .host_config = {
                .host_connection_mode = HOST_CONNECTION_MODE_NONE,
            },
            .port_config = {
                .storage_partition_name = "nvs",
                .netif_queue_size = 10,
                .task_queue_size = 10,
            },
        },
    };
    result = esp_openthread_start(&config);
    printf("REMU_VENDOR_THREAD_START result=%d\n", (int)result);
    if (result != ESP_OK) {
        return;
    }

    esp_openthread_lock_acquire(portMAX_DELAY);
    otInstance *instance = esp_openthread_get_instance();
    otCliInit(instance, cli_output, NULL);
    esp_openthread_lock_release();

    run_cli("dataset init new");
    run_cli("dataset activetimestamp 1");
    run_cli("dataset channel 11");
    run_cli("dataset extpanid 52454d552d544852");
    run_cli("dataset meshlocalprefix fd00:db8:7265:6d75::");
    run_cli("dataset networkkey 000102030405060708090a0b0c0d0e0f");
    run_cli("dataset networkname REMU-THREAD");
    run_cli("dataset panid 0x1234");
    run_cli("dataset commit active");
    run_cli("routerselectionjitter 1");
    run_cli("ifconfig up");
    start_as_leader(instance);
}
