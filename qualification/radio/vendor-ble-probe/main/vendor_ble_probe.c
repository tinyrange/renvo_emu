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
    }
    return 0;
}

static void on_sync(void)
{
    static const uint8_t random_static_address[6] = {
        0x06, 0x05, 0x04, 0x03, 0x02, 0xc1,
    };
    static const uint8_t advertisement[] = {
        0x02, BLE_HS_ADV_TYPE_FLAGS,
        BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP,
        0x0b, BLE_HS_ADV_TYPE_COMP_NAME,
        'R', 'e', 'n', 'v', 'o', '-', 'B', 'L', 'E', '1',
    };
    struct ble_gap_adv_params parameters = {0};
    struct ble_gap_disc_params discovery = {0};

    int result = ble_hs_id_set_rnd(random_static_address);
    if (result == 0) {
        result = ble_gap_adv_set_data(advertisement, sizeof(advertisement));
    }
    parameters.conn_mode = BLE_GAP_CONN_MODE_NON;
    parameters.disc_mode = BLE_GAP_DISC_MODE_GEN;
    if (result == 0) {
        result = ble_gap_adv_start(BLE_OWN_ADDR_RANDOM, NULL, BLE_HS_FOREVER,
                                   &parameters, NULL, NULL);
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
    int stop_result = ble_gap_adv_stop();
    printf("REMU_VENDOR_BLE_ADV_STOP result=%d\n", stop_result);
    fflush(stdout);

    discovery.itvl = 16;
    discovery.window = 16;
    discovery.passive = 1;
    discovery.filter_duplicates = 0;
    int scan_result = ble_gap_disc(BLE_OWN_ADDR_RANDOM, BLE_HS_FOREVER,
                                   &discovery, on_gap_event, NULL);
    printf("REMU_VENDOR_BLE_SCAN_START result=%d\n", scan_result);
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
