/* SPDX-License-Identifier: Apache-2.0 */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#include "esp_ieee802154.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

static volatile bool transmit_complete;
static volatile bool transmit_failed;
static volatile bool receive_complete;
static volatile bool energy_detect_complete;
static volatile uint8_t transmitted_frame[5];
static volatile uint8_t received_frame[16];
static volatile int transmit_error;
static volatile int8_t received_rssi;
static volatile uint8_t received_lqi;
static volatile unsigned received_mpf_index;
static volatile bool received_pending;
static volatile int8_t detected_power;
static volatile bool ack_received;
static volatile unsigned receive_callback_count;
static volatile uint8_t received_ack[6];
static volatile int8_t ack_rssi;
static volatile uint8_t ack_lqi;

static uint8_t transmit_frame[] = {4, 0x01, 0x00, 0x2a, 0xa5};
static uint8_t ack_transmit_frame[] = {4, 0x21, 0x00, 0x44, 0xa5};
static uint8_t no_ack_transmit_frame[] = {4, 0x21, 0x00, 0x45, 0xa5};

void esp_ieee802154_transmit_done(const uint8_t *frame, const uint8_t *ack,
                                  esp_ieee802154_frame_info_t *ack_info)
{
    for (unsigned index = 0; index < sizeof(transmitted_frame); ++index) {
        transmitted_frame[index] = frame[index];
    }
    if (ack != NULL && ack_info != NULL) {
        for (unsigned index = 0; index < sizeof(received_ack); ++index) {
            received_ack[index] = ack[index];
        }
        ack_rssi = ack_info->rssi;
        ack_lqi = ack_info->lqi;
        ack_received = true;
        esp_ieee802154_receive_handle_done(ack);
    }
    transmit_complete = true;
}

void esp_ieee802154_transmit_failed(const uint8_t *frame,
                                    esp_ieee802154_tx_error_t error)
{
    (void)frame;
    transmit_error = (int)error;
    transmit_failed = true;
}

void esp_ieee802154_receive_done(uint8_t *frame,
                                 esp_ieee802154_frame_info_t *frame_info)
{
    ++receive_callback_count;
    for (unsigned index = 0; index < sizeof(received_frame); ++index) {
        received_frame[index] = frame[index];
    }
    received_rssi = frame_info->rssi;
    received_lqi = frame_info->lqi;
    received_mpf_index = (unsigned)frame_info->mpf_index;
    received_pending = frame_info->pending;
    receive_complete = true;
    esp_ieee802154_receive_handle_done(frame);
}

void esp_ieee802154_receive_sfd_done(void)
{
}

void esp_ieee802154_transmit_sfd_done(uint8_t *frame)
{
    (void)frame;
}

void esp_ieee802154_energy_detect_done(int8_t power)
{
    detected_power = power;
    energy_detect_complete = true;
}

void esp_ieee802154_receive_at_done(void)
{
}

void app_main(void)
{
    esp_err_t result = esp_ieee802154_enable();
    printf("REMU_VENDOR_IEEE802154_INIT result=%d\n", (int)result);
    if (result != ESP_OK) {
        return;
    }

    result = esp_ieee802154_set_channel(11);
    if (result == ESP_OK) {
        result = esp_ieee802154_set_promiscuous(true);
    }
    printf("REMU_VENDOR_IEEE802154_CONFIG result=%d channel=%u promiscuous=%u\n",
           (int)result, esp_ieee802154_get_channel(),
           esp_ieee802154_get_promiscuous() ? 1u : 0u);
    if (result != ESP_OK) {
        return;
    }

    result = esp_ieee802154_transmit(transmit_frame, false);
    printf("REMU_VENDOR_IEEE802154_TX_START result=%d\n", (int)result);
    for (unsigned timeout = 0;
         result == ESP_OK && !transmit_complete && !transmit_failed && timeout < 200;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (transmit_failed) {
        printf("REMU_VENDOR_IEEE802154_TX_FAILED error=%d\n", transmit_error);
        return;
    }
    if (!transmit_complete) {
        printf("REMU_VENDOR_IEEE802154_TX_TIMEOUT\n");
        return;
    }
    printf("REMU_VENDOR_IEEE802154_TX_DONE length=%u %02x %02x %02x %02x\n",
           transmitted_frame[0], transmitted_frame[1], transmitted_frame[2],
           transmitted_frame[3], transmitted_frame[4]);

    transmit_complete = false;
    transmit_failed = false;
    transmit_error = -1;
    result = esp_ieee802154_transmit(transmit_frame, true);
    printf("REMU_VENDOR_IEEE802154_CCA_TX_START result=%d\n", (int)result);
    for (unsigned timeout = 0;
         result == ESP_OK && !transmit_complete && !transmit_failed && timeout < 200;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    printf("REMU_VENDOR_IEEE802154_CCA_TX_DONE complete=%u failed=%u error=%d\n",
           transmit_complete ? 1u : 0u, transmit_failed ? 1u : 0u,
           transmit_error);
    if (!transmit_complete || transmit_failed) {
        return;
    }

    result = esp_ieee802154_receive();
    printf("REMU_VENDOR_IEEE802154_RX_START result=%d\n", (int)result);
    for (unsigned timeout = 0; result == ESP_OK && !receive_complete && timeout < 400; ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (!receive_complete) {
        printf("REMU_VENDOR_IEEE802154_RX_TIMEOUT\n");
        return;
    }
    printf("REMU_VENDOR_IEEE802154_RX_DONE length=%u %02x %02x %02x %02x rssi=%d lqi=%u\n",
           received_frame[0], received_frame[1], received_frame[2], received_frame[3],
           received_frame[4], (int)received_rssi, (unsigned)received_lqi);

    result = esp_ieee802154_energy_detect(8);
    printf("REMU_VENDOR_IEEE802154_ED_START result=%d\n", (int)result);
    for (unsigned timeout = 0; result == ESP_OK && !energy_detect_complete && timeout < 200; ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (!energy_detect_complete) {
        printf("REMU_VENDOR_IEEE802154_ED_TIMEOUT\n");
        return;
    }
    printf("REMU_VENDOR_IEEE802154_ED_DONE power=%d\n", (int)detected_power);

    result = esp_ieee802154_set_promiscuous(false);
    if (result == ESP_OK) {
        result = esp_ieee802154_set_multipan_panid(ESP_IEEE802154_MULTIPAN_0,
                                                   0x1234);
    }
    if (result == ESP_OK) {
        result = esp_ieee802154_set_multipan_short_address(
            ESP_IEEE802154_MULTIPAN_0, 0x5678);
    }
    if (result == ESP_OK) {
        result = esp_ieee802154_set_multipan_panid(ESP_IEEE802154_MULTIPAN_1,
                                                   0xabcd);
    }
    if (result == ESP_OK) {
        result = esp_ieee802154_set_multipan_short_address(
            ESP_IEEE802154_MULTIPAN_1, 0x1357);
    }
    if (result == ESP_OK) {
        result = esp_ieee802154_set_multipan_enable(
            (1u << ESP_IEEE802154_MULTIPAN_0) |
            (1u << ESP_IEEE802154_MULTIPAN_1));
    }
    printf("REMU_VENDOR_IEEE802154_MULTIPAN result=%d mask=%u pan0=%04x short0=%04x pan1=%04x short1=%04x\n",
           (int)result, (unsigned)esp_ieee802154_get_multipan_enable(),
           (unsigned)esp_ieee802154_get_multipan_panid(ESP_IEEE802154_MULTIPAN_0),
           (unsigned)esp_ieee802154_get_multipan_short_address(ESP_IEEE802154_MULTIPAN_0),
           (unsigned)esp_ieee802154_get_multipan_panid(ESP_IEEE802154_MULTIPAN_1),
           (unsigned)esp_ieee802154_get_multipan_short_address(ESP_IEEE802154_MULTIPAN_1));
    if (result != ESP_OK) {
        return;
    }

    receive_complete = false;
    result = esp_ieee802154_receive();
    printf("REMU_VENDOR_IEEE802154_FILTER_RX_START result=%d\n", (int)result);
    if (result == ESP_OK) {
        /* The rejected frame ends the native one-shot RX operation.  Leave a
         * deliberate interval for its RX_ABORT interrupt before re-arming. */
        vTaskDelay(pdMS_TO_TICKS(50));
        result = esp_ieee802154_receive();
        printf("REMU_VENDOR_IEEE802154_FILTER_RX_REARM result=%d\n",
               (int)result);
    }
    for (unsigned timeout = 0; result == ESP_OK && !receive_complete && timeout < 500;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (!receive_complete) {
        printf("REMU_VENDOR_IEEE802154_FILTER_RX_TIMEOUT\n");
        return;
    }
    printf("REMU_VENDOR_IEEE802154_FILTER_RX_DONE length=%u %02x %02x %02x %02x %02x %02x %02x %02x %02x mpf=%u rssi=%d lqi=%u\n",
           received_frame[0], received_frame[1], received_frame[2],
           received_frame[3], received_frame[4], received_frame[5],
           received_frame[6], received_frame[7], received_frame[8],
           received_frame[9], received_mpf_index, (int)received_rssi,
           (unsigned)received_lqi);

    receive_complete = false;
    received_pending = false;
    result = esp_ieee802154_receive();
    printf("REMU_VENDOR_IEEE802154_AUTO_ACK_RX_START result=%d\n", (int)result);
    for (unsigned timeout = 0; result == ESP_OK && !receive_complete && timeout < 500;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (!receive_complete) {
        printf("REMU_VENDOR_IEEE802154_AUTO_ACK_RX_TIMEOUT callbacks=%u\n",
               receive_callback_count);
        return;
    }
    printf("REMU_VENDOR_IEEE802154_AUTO_ACK_RX_DONE length=%u %02x %02x %02x %02x %02x %02x %02x %02x %02x pending=%u rssi=%d lqi=%u callbacks=%u\n",
           received_frame[0], received_frame[1], received_frame[2],
           received_frame[3], received_frame[4], received_frame[5],
           received_frame[6], received_frame[7], received_frame[8],
           received_frame[9], received_pending ? 1u : 0u,
           (int)received_rssi, (unsigned)received_lqi,
           receive_callback_count);

    transmit_complete = false;
    transmit_failed = false;
    ack_received = false;
    result = esp_ieee802154_transmit(ack_transmit_frame, false);
    printf("REMU_VENDOR_IEEE802154_ACK_TX_START result=%d\n", (int)result);
    for (unsigned timeout = 0;
         result == ESP_OK && !transmit_complete && !transmit_failed && timeout < 500;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (!transmit_complete || transmit_failed || !ack_received) {
        printf("REMU_VENDOR_IEEE802154_ACK_TX_FAILED complete=%u failed=%u ack=%u error=%d\n",
               transmit_complete ? 1u : 0u, transmit_failed ? 1u : 0u,
               ack_received ? 1u : 0u, transmit_error);
        return;
    }
    printf("REMU_VENDOR_IEEE802154_ACK_RX_DONE length=%u %02x %02x %02x rssi=%d lqi=%u\n",
           received_ack[0], received_ack[1], received_ack[2], received_ack[3],
           (int)ack_rssi, (unsigned)ack_lqi);

    transmit_complete = false;
    transmit_failed = false;
    transmit_error = -1;
    result = esp_ieee802154_transmit(no_ack_transmit_frame, false);
    printf("REMU_VENDOR_IEEE802154_NO_ACK_TX_START result=%d\n", (int)result);
    for (unsigned timeout = 0;
         result == ESP_OK && !transmit_complete && !transmit_failed && timeout < 500;
         ++timeout) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    printf("REMU_VENDOR_IEEE802154_NO_ACK_DONE complete=%u failed=%u error=%d\n",
           transmit_complete ? 1u : 0u, transmit_failed ? 1u : 0u,
           transmit_error);

}
