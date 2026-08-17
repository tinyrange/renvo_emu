/* SPDX-License-Identifier: Apache-2.0 */
#include "dma.h"
#include "mac.h"
#include "mmio.h"
#include "rf.h"

#define WIFI_MAC_BASE 0x600a4000u
#define WIFI_INTERRUPT_MASK (WIFI_MAC_BASE + 0x0c40u)
#define WIFI_INTERRUPT_EVENT (WIFI_MAC_BASE + 0x0c48u)
#define WIFI_INTERRUPT_CLEAR (WIFI_MAC_BASE + 0x0c4cu)
#define WIFI_RX_BASE (WIFI_MAC_BASE + 0x0084u)
#define WIFI_QUEUE_STATE_CLEAR (WIFI_MAC_BASE + 0x0cb4u)
#define WIFI_QUEUE_STATE (WIFI_MAC_BASE + 0x0cb8u)
#define WIFI_QUEUE2_CONTROL (WIFI_MAC_BASE + 0x0d6cu)
#define WIFI_TX_DONE (1u << 7)
#define WIFI_RX_DONE (1u << 14)
#define WIFI_QUEUE_ENABLE (3u << 30)
#define INTERRUPT_MATRIX_WIFI_ROUTE 0x60010000u
#define WIFI_CPU_INTERRUPT 5u
#define PLIC_MACHINE_BASE 0x20001000u
#define PLIC_ENABLE (PLIC_MACHINE_BASE + 0x0000u)
#define PLIC_PRIORITY(line) (PLIC_MACHINE_BASE + 0x0010u + (line) * 4u)
#define PLIC_THRESHOLD (PLIC_MACHINE_BASE + 0x0090u)
#define WIFI_RX_CAPACITY 512u
#define WIFI_RX_METADATA 92u

uint8_t c6_rf_probe_tx_buffer[96] __attribute__((aligned(4)));
struct c6_dma_descriptor c6_rf_probe_tx_descriptor __attribute__((aligned(4)));
uint8_t c6_rf_probe_rx_buffer[WIFI_RX_CAPACITY] __attribute__((aligned(4)));
struct c6_dma_descriptor c6_rf_probe_rx_descriptor __attribute__((aligned(4)));
static uint16_t sequence_number;

static uint32_t append_tag(uint8_t *destination, const char *tag)
{
    static const char prefix[] = "REMU-C6-RF-";
    uint32_t length = 0;
    for (uint32_t index = 0; index < sizeof(prefix) - 1u; ++index) {
        destination[length++] = (uint8_t)prefix[index];
    }
    while (*tag != '\0' && length < 32u) {
        destination[length++] = (uint8_t)*tag++;
    }
    return length;
}

static uint32_t make_probe_request(const char *tag)
{
    uint8_t *frame = c6_rf_probe_tx_buffer + 8u;
    static const uint8_t header[22] = {
        0x40,0x00,0x00,0x00,
        0xff,0xff,0xff,0xff,0xff,0xff,
        0x02,0x00,0x00,0x00,0x00,0xc6,
        0xff,0xff,0xff,0xff,0xff,0xff,
    };
    for (uint32_t index = 0; index < sizeof(header); ++index) {
        frame[index] = header[index];
    }
    frame[22] = (uint8_t)(sequence_number << 4);
    frame[23] = (uint8_t)(sequence_number >> 4);
    sequence_number = (sequence_number + 1u) & 0x0fffu;
    frame[24] = 0;
    uint32_t ssid_length = append_tag(frame + 26u, tag);
    frame[25] = (uint8_t)ssid_length;
    uint32_t frame_length = 26u + ssid_length;
    c6_rf_probe_tx_buffer[0] = (uint8_t)(frame_length + 4u);
    for (uint32_t index = 1; index < 8u; ++index) {
        c6_rf_probe_tx_buffer[index] = 0;
    }
    return frame_length;
}

int c6_mac_init(void)
{
    c6_write32(INTERRUPT_MATRIX_WIFI_ROUTE, WIFI_CPU_INTERRUPT);
    c6_write32(PLIC_ENABLE, c6_read32(PLIC_ENABLE) | (1u << WIFI_CPU_INTERRUPT));
    c6_write32(PLIC_PRIORITY(WIFI_CPU_INTERRUPT), 3u);
    c6_write32(PLIC_THRESHOLD, 1u);
    c6_write32(WIFI_INTERRUPT_MASK, WIFI_TX_DONE | WIFI_RX_DONE);
    c6_write32(WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE | WIFI_RX_DONE);
    c6_write32(WIFI_QUEUE_STATE_CLEAR, 0xffffffffu);
    return 0;
}

int c6_mac_tx_probe(const char *tag)
{
    if (!c6_rf_ready()) return -1;
    (void)make_probe_request(tag);
    c6_dma_tx_descriptor(&c6_rf_probe_tx_descriptor, c6_rf_probe_tx_buffer);
    c6_write32(WIFI_QUEUE2_CONTROL,
               WIFI_QUEUE_ENABLE |
               ((uint32_t)(uintptr_t)&c6_rf_probe_tx_descriptor & 0x000fffffu));
    for (uint32_t timeout = 0; timeout < 50000u; ++timeout) {
        uint32_t event = c6_read32(WIFI_INTERRUPT_EVENT);
        if ((event & WIFI_TX_DONE) != 0 &&
            (c6_read32(WIFI_QUEUE_STATE) & 1u) != 0) {
            c6_write32(WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE);
            c6_write32(WIFI_QUEUE_STATE_CLEAR, 1u);
            return 0;
        }
    }
    return -2;
}

void c6_mac_rx_start(void)
{
    c6_rf_rx_enable(1);
    c6_dma_rx_descriptor(&c6_rf_probe_rx_descriptor,
                         c6_rf_probe_rx_buffer, WIFI_RX_CAPACITY);
    c6_write32(WIFI_RX_BASE, (uint32_t)(uintptr_t)&c6_rf_probe_rx_descriptor);
}

void c6_mac_rx_stop(void)
{
    c6_write32(WIFI_RX_BASE, 0);
    c6_rf_rx_enable(0);
}

int c6_mac_rx_poll(uint32_t *wire_length, int8_t *rssi)
{
    if ((c6_read32(WIFI_INTERRUPT_EVENT) & WIFI_RX_DONE) == 0) return 0;
    uint32_t control = c6_rf_probe_rx_descriptor.control;
    uint32_t length = (control >> 14) & 0x3fffu;
    if ((control & (1u << 31)) != 0 || (control & (1u << 30)) == 0 ||
        length < WIFI_RX_METADATA + 4u || length > WIFI_RX_CAPACITY) {
        return -1;
    }
    *wire_length = length - WIFI_RX_METADATA;
    *rssi = (int8_t)c6_rf_probe_rx_buffer[11];
    c6_write32(WIFI_INTERRUPT_CLEAR, WIFI_RX_DONE);
    c6_mac_rx_start();
    return 1;
}
