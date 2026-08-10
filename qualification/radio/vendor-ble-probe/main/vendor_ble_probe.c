/*
 * Project-owned qualification firmware for the genuine ESP-IDF BLE stack.
 *
 * Only public NimBLE APIs are used. Renvo must execute the linked controller,
 * ROM, PHY, scheduler, and RTOS code without symbol or address interception.
 */

#include <stdio.h>
#include <string.h>

#include "host/ble_hs.h"
#include "host/ble_hs_adv.h"
#include "host/ble_gap.h"
#include "host/ble_hs_mbuf.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static void host_task(void *argument)
{
    (void)argument;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

static void on_reset(int reason)
{
    printf("REMU_VENDOR_BLE_RESET reason=%d\n", reason);
    fflush(stdout);
}

static int on_gap_event(struct ble_gap_event *event, void *argument)
{
    (void)argument;
    if (event->type == BLE_GAP_EVENT_DISC) {
        const struct ble_gap_disc_desc *report = &event->disc;
        printf("REMU_VENDOR_BLE_SCAN_REPORT type=%u length=%u rssi=%d",
               report->event_type, report->length_data, report->rssi);
        for (uint8_t index = 0; index < report->length_data; ++index) {
            printf(" %02x", report->data[index]);
        }
        printf("\n");
        fflush(stdout);
    } else if (event->type == BLE_GAP_EVENT_EXT_DISC) {
        const struct ble_gap_ext_disc_desc *report = &event->ext_disc;
        printf("REMU_VENDOR_BLE_EXT_SCAN_REPORT props=%u status=%u legacy=%u "
               "length=%u rssi=%d",
               report->props, report->data_status, report->legacy_event_type,
               report->length_data, report->rssi);
        for (uint8_t index = 0; index < report->length_data; ++index) {
            printf(" %02x", report->data[index]);
        }
        printf("\n");
        fflush(stdout);
    }
    return 0;
}

static int configure_advertising(const uint8_t *advertisement,
                                 size_t advertisement_length,
                                 bool legacy_pdu)
{
    struct ble_gap_ext_adv_params parameters = {0};
    struct os_mbuf *data;
    const uint8_t instance = 0;
    int result;

    parameters.legacy_pdu = legacy_pdu;
    parameters.own_addr_type = BLE_OWN_ADDR_RANDOM;
    parameters.primary_phy = BLE_HCI_LE_PHY_1M;
    parameters.secondary_phy = BLE_HCI_LE_PHY_2M;
    parameters.tx_power = 127;
    parameters.sid = legacy_pdu ? 0 : 3;
    parameters.itvl_min = BLE_GAP_ADV_ITVL_MS(20);
    parameters.itvl_max = BLE_GAP_ADV_ITVL_MS(20);

    result = ble_gap_ext_adv_configure(instance, &parameters, NULL,
                                       on_gap_event, NULL);
    if (result != 0) {
        return result;
    }

    data = ble_hs_mbuf_from_flat(advertisement, advertisement_length);
    if (data == NULL) {
        return BLE_HS_ENOMEM;
    }
    result = ble_gap_ext_adv_set_data(instance, data);
    if (result != 0) {
        return result;
    }

    return ble_gap_ext_adv_start(instance, 0, 0);
}

static int run_extended_advertising(void)
{
    static const uint8_t extended_advertisement[] = {
        0x02, BLE_HS_ADV_TYPE_FLAGS,
        BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP,
        0x0a, BLE_HS_ADV_TYPE_COMP_NAME,
        'R', 'e', 'n', 'v', 'o', '-', 'E', 'X', 'T',
    };
    return configure_advertising(extended_advertisement,
                                 sizeof(extended_advertisement), false);
}

static void radio_sequence_task(void *argument)
{
    (void)argument;
    static const uint8_t random_static_address[6] = {
        0x06, 0x05, 0x04, 0x03, 0x02, 0xc1,
    };
    static const uint8_t advertisement[] = {
        0x02, BLE_HS_ADV_TYPE_FLAGS,
        BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP,
        0x0b, BLE_HS_ADV_TYPE_COMP_NAME,
        'R', 'e', 'n', 'v', 'o', '-', 'B', 'L', 'E', '1',
    };
    struct ble_gap_disc_params discovery = {0};

    int result = ble_hs_id_set_rnd(random_static_address);
    if (result == 0) {
        result = configure_advertising(advertisement,
                                       sizeof(advertisement), true);
    }

    printf("REMU_VENDOR_BLE_ADV_START result=%d\n", result);
    fflush(stdout);

    /*
     * Qualify TX and RX as separate native controller lifecycles. Keeping the
     * advertisement alive for 20 ms gives the baseband time to emit a full
     * three-channel event; stopping it then also exercises the vendor abort
     * path before the scanner takes ownership of the same scheduler.
     */
    vTaskDelay(pdMS_TO_TICKS(20));
    int stop_result = ble_gap_ext_adv_stop(0);
    printf("REMU_VENDOR_BLE_ADV_STOP result=%d\n", stop_result);
    fflush(stdout);

    vTaskDelay(pdMS_TO_TICKS(10));
    int remove_result = ble_gap_ext_adv_remove(0);
    printf("REMU_VENDOR_BLE_ADV_REMOVE result=%d\n", remove_result);
    fflush(stdout);

    int extended_result = run_extended_advertising();
    printf("REMU_VENDOR_BLE_EXT_ADV_START result=%d\n", extended_result);
    fflush(stdout);

    vTaskDelay(pdMS_TO_TICKS(20));
    int extended_stop_result = ble_gap_ext_adv_stop(0);
    printf("REMU_VENDOR_BLE_EXT_ADV_STOP result=%d\n", extended_stop_result);
    fflush(stdout);

    vTaskDelay(pdMS_TO_TICKS(10));
    int extended_remove_result = ble_gap_ext_adv_remove(0);
    printf("REMU_VENDOR_BLE_EXT_ADV_REMOVE result=%d\n",
           extended_remove_result);
    fflush(stdout);

    discovery.itvl = 16;
    discovery.window = 16;
    discovery.passive = 1;
    discovery.filter_duplicates = 0;
    int scan_result = ble_gap_disc(BLE_OWN_ADDR_RANDOM, BLE_HS_FOREVER,
                                   &discovery, on_gap_event, NULL);
    printf("REMU_VENDOR_BLE_SCAN_START result=%d\n", scan_result);
    fflush(stdout);
    vTaskDelete(NULL);
}

static void on_sync(void)
{
    BaseType_t result = xTaskCreate(radio_sequence_task, "radio-sequence",
                                    4096, NULL, 5, NULL);
    printf("REMU_VENDOR_BLE_SEQUENCE result=%ld\n", (long)result);
    fflush(stdout);
}

void app_main(void)
{
    int result = nimble_port_init();
    printf("REMU_VENDOR_BLE_INIT result=%d\n", result);
    fflush(stdout);
    if (result != 0) {
        return;
    }

    ble_hs_cfg.reset_cb = on_reset;
    ble_hs_cfg.sync_cb = on_sync;
    nimble_port_freertos_init(host_task);
}
