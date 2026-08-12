/*
 * Project-owned qualification firmware for genuine ESP-IDF Wi-Fi libraries.
 *
 * Only public ESP-IDF APIs are used. Renvo executes the linked ROM, PHY,
 * net80211, ESP-NOW, and RTOS code without symbol interception.
 */

#include <stdio.h>
#include <string.h>

#include "esp_err.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "esp_now.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/inet.h"
#include "lwip/sockets.h"

static volatile bool soft_ap_started;
static volatile bool soft_ap_station_connected;
static volatile bool esp_now_received;

static unsigned send_udp_burst(void)
{
    int socket_fd = socket(AF_INET, SOCK_DGRAM, IPPROTO_IP);
    if (socket_fd < 0) {
        return 0;
    }
    struct sockaddr_in destination = {
        .sin_family = AF_INET,
        .sin_port = htons(4242),
        .sin_addr.s_addr = inet_addr("192.168.4.2"),
    };
    const uint8_t prime[] = {0x52, 0x45, 0x4d, 0x55};
    (void)sendto(socket_fd, prime, sizeof(prime), 0,
                 (const struct sockaddr *)&destination, sizeof(destination));
    vTaskDelay(pdMS_TO_TICKS(100));

    /* Keep several MPDUs inside the peer's bounded HT receive limit. */
    uint8_t payload[256];
    memset(payload, 0x5a, sizeof(payload));
    unsigned sent = 0;
    for (unsigned packet = 0; packet < 64; ++packet) {
        payload[0] = (uint8_t)packet;
        if (sendto(socket_fd, payload, sizeof(payload), 0,
                   (const struct sockaddr *)&destination,
                   sizeof(destination)) == sizeof(payload)) {
            ++sent;
        }
    }
    close(socket_fd);
    return sent;
}

static void print_mac(const uint8_t address[6])
{
    printf("%02x:%02x:%02x:%02x:%02x:%02x",
           address[0], address[1], address[2],
           address[3], address[4], address[5]);
}

static void on_wifi_event(void *argument, esp_event_base_t event_base,
                          int32_t event_id, void *event_data)
{
    (void)argument;
    (void)event_base;
    if (event_id == WIFI_EVENT_AP_START) {
        soft_ap_started = true;
        printf("REMU_VENDOR_SOFTAP_STARTED\n");
    } else if (event_id == WIFI_EVENT_AP_STACONNECTED) {
        const wifi_event_ap_staconnected_t *event = event_data;
        soft_ap_station_connected = true;
        printf("REMU_VENDOR_SOFTAP_STATION_CONNECTED mac=");
        print_mac(event->mac);
        printf(" aid=%u\n", (unsigned)event->aid);
    } else if (event_id == WIFI_EVENT_AP_STADISCONNECTED) {
        const wifi_event_ap_stadisconnected_t *event = event_data;
        soft_ap_station_connected = false;
        printf("REMU_VENDOR_SOFTAP_STATION_DISCONNECTED mac=");
        print_mac(event->mac);
        printf(" aid=%u reason=%u\n", (unsigned)event->aid,
               (unsigned)event->reason);
    }
    fflush(stdout);
}

static void on_esp_now_receive(const esp_now_recv_info_t *information,
                               const uint8_t *data, int length)
{
    esp_now_received = true;
    printf("REMU_VENDOR_ESPNOW_RX source=");
    print_mac(information->src_addr);
    printf(" destination=");
    print_mac(information->des_addr);
    printf(" length=%d data=", length);
    for (int index = 0; index < length; ++index) {
        printf("%02x", data[index]);
    }
    printf("\n");
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
            .ssid = "REMU-SOFTAP",
            .ssid_len = 11,
            .channel = 1,
            .authmode = WIFI_AUTH_OPEN,
            .max_connection = 4,
            .beacon_interval = 100,
        },
    };
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &configuration));
#if CONFIG_SOC_WIFI_HE_SUPPORT
    const uint8_t he_protocols = WIFI_PROTOCOL_11B | WIFI_PROTOCOL_11G |
                                 WIFI_PROTOCOL_11N | WIFI_PROTOCOL_11AX;
    ESP_ERROR_CHECK(esp_wifi_set_protocol(WIFI_IF_AP, he_protocols));
#endif
    ESP_ERROR_CHECK(esp_wifi_start());

    uint8_t ap_address[6];
    uint8_t protocols = 0;
    ESP_ERROR_CHECK(esp_wifi_get_mac(WIFI_IF_AP, ap_address));
    ESP_ERROR_CHECK(esp_wifi_get_protocol(WIFI_IF_AP, &protocols));
    printf("REMU_VENDOR_SOFTAP_CONFIG mac=%02x:%02x:%02x:%02x:%02x:%02x"
           " channel=1 ssid=REMU-SOFTAP protocols=0x%02x\n",
           ap_address[0], ap_address[1], ap_address[2], ap_address[3],
           ap_address[4], ap_address[5], (unsigned)protocols);
    fflush(stdout);

    ESP_ERROR_CHECK(esp_now_init());
    ESP_ERROR_CHECK(esp_now_register_recv_cb(on_esp_now_receive));
    const uint8_t broadcast[6] = {0xff, 0xff, 0xff, 0xff, 0xff, 0xff};
    esp_now_peer_info_t peer = {
        .channel = 1,
        .ifidx = WIFI_IF_AP,
        .encrypt = false,
    };
    memcpy(peer.peer_addr, broadcast, sizeof(peer.peer_addr));
    ESP_ERROR_CHECK(esp_now_add_peer(&peer));

    const uint8_t secure_station[6] = {0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01};
    const uint8_t secure_lmk[ESP_NOW_KEY_LEN] = {
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    };
    esp_now_peer_info_t secure_peer = {
        .channel = 1,
        .ifidx = WIFI_IF_AP,
        .encrypt = true,
    };
    memcpy(secure_peer.peer_addr, secure_station, sizeof(secure_peer.peer_addr));
    memcpy(secure_peer.lmk, secure_lmk, sizeof(secure_peer.lmk));
    ESP_ERROR_CHECK(esp_now_add_peer(&secure_peer));

    const uint8_t payload[] = {0x52, 0x45, 0x4d, 0x55};
    esp_err_t send_result = esp_now_send(broadcast, payload, sizeof(payload));
    printf("REMU_VENDOR_ESPNOW_TX_START result=%d\n", (int)send_result);
    fflush(stdout);

    const uint8_t secure_payload[] = {0x43, 0x43, 0x4d, 0x50};
    esp_err_t secure_send_result =
        esp_now_send(secure_station, secure_payload, sizeof(secure_payload));
    printf("REMU_VENDOR_ESPNOW_SECURE_TX_START result=%d\n",
           (int)secure_send_result);
    fflush(stdout);

    for (unsigned attempt = 0;
         attempt < 400 && (!soft_ap_station_connected || !esp_now_received);
         ++attempt) {
        vTaskDelay(pdMS_TO_TICKS(10));
    }
    unsigned udp_packets = 0;
    if (soft_ap_station_connected) {
        udp_packets = send_udp_burst();
        vTaskDelay(pdMS_TO_TICKS(500));
    }
    printf("REMU_VENDOR_WIFI_UDP_BURST packets=%u\n", udp_packets);
    printf("REMU_VENDOR_SOFTAP_DONE started=%u station=%u espnow_rx=%u\n",
           soft_ap_started ? 1u : 0u,
           soft_ap_station_connected ? 1u : 0u,
           esp_now_received ? 1u : 0u);
    fflush(stdout);
}
