/* SPDX-License-Identifier: Apache-2.0 */
#include "dma.h"
#include "mac.h"
#include "mmio.h"
#include "registers.h"
#include "rf.h"

#define WIFI_TX_DONE (1u << 7)
#define WIFI_RX_DONE (1u << 14)
#define WIFI_QUEUE_ENABLE (3u << 30)
#define WIFI_CPU_INTERRUPT 5u
#define WIFI_RX_CAPACITY 512u
#define WIFI_RX_METADATA 92u
#define WIFI_INTERFACE_ADDRESS_VALID (1u << 16)

uint8_t c6_rf_probe_tx_buffer[256] __attribute__((aligned(4)));
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
    c6_write32(C6_REG_INTERRUPT_MATRIX_WIFI_ROUTE, WIFI_CPU_INTERRUPT);
    c6_write32(C6_REG_PLIC_ENABLE,
               c6_read32(C6_REG_PLIC_ENABLE) | (1u << WIFI_CPU_INTERRUPT));
    c6_write32(C6_REG_PLIC_WIFI_PRIORITY, 3u);
    c6_write32(C6_REG_PLIC_THRESHOLD, 1u);
    c6_write32(C6_REG_WIFI_INTERRUPT_MASK, WIFI_TX_DONE | WIFI_RX_DONE);
    c6_write32(C6_REG_WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE | WIFI_RX_DONE);
    c6_write32(C6_REG_WIFI_TX_QUEUE_STATE_CLEAR, 0xffffffffu);
    return C6_MAC_OK;
}

int c6_mac_tx_probe(const char *tag)
{
    if (!c6_rf_ready()) return C6_MAC_NOT_READY;
    (void)make_probe_request(tag);
    return c6_mac_tx_frame(c6_rf_probe_tx_buffer + 8u,
                           (uint32_t)c6_rf_probe_tx_buffer[0] - 4u);
}

int c6_mac_tx_frame(const uint8_t *frame, uint32_t length)
{
    if (!c6_rf_ready()) return C6_MAC_NOT_READY;
    if (length < 10u || length + 12u > sizeof(c6_rf_probe_tx_buffer)) {
        return C6_MAC_INVALID_FRAME;
    }
    c6_rf_probe_tx_buffer[0] = (uint8_t)(length + 4u);
    for (uint32_t index = 1; index < 8u; ++index) {
        c6_rf_probe_tx_buffer[index] = 0;
    }
    if (frame != c6_rf_probe_tx_buffer + 8u) {
        for (uint32_t index = 0; index < length; ++index) {
            c6_rf_probe_tx_buffer[8u + index] = frame[index];
        }
    }
    c6_dma_tx_descriptor(&c6_rf_probe_tx_descriptor, c6_rf_probe_tx_buffer);
    c6_write32(C6_REG_WIFI_TX_QUEUE0_CONTROL,
               WIFI_QUEUE_ENABLE |
               ((uint32_t)(uintptr_t)&c6_rf_probe_tx_descriptor & 0x000fffffu));
    for (uint32_t timeout = 0; timeout < 50000u; ++timeout) {
        uint32_t event = c6_read32(C6_REG_WIFI_INTERRUPT_EVENT);
        if ((event & WIFI_TX_DONE) != 0 &&
            (c6_read32(C6_REG_WIFI_TX_QUEUE_STATE) & 1u) != 0) {
            c6_write32(C6_REG_WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE);
            c6_write32(C6_REG_WIFI_TX_QUEUE_STATE_CLEAR, 1u);
            return C6_MAC_OK;
        }
    }
    return C6_MAC_TIMEOUT;
}

void c6_mac_rx_start(void)
{
    c6_rf_rx_enable(1);
    c6_dma_rx_descriptor(&c6_rf_probe_rx_descriptor,
                         c6_rf_probe_rx_buffer, WIFI_RX_CAPACITY);
    c6_write32(C6_REG_WIFI_RX_BASE, (uint32_t)(uintptr_t)&c6_rf_probe_rx_descriptor);
}

void c6_mac_rx_stop(void)
{
    c6_write32(C6_REG_WIFI_RX_BASE, 0);
    c6_rf_rx_enable(0);
}

int c6_mac_rx_poll(uint32_t *wire_length, int8_t *rssi)
{
    if ((c6_read32(C6_REG_WIFI_INTERRUPT_EVENT) & WIFI_RX_DONE) == 0) {
        return C6_MAC_NO_PACKET;
    }
    uint32_t control = c6_rf_probe_rx_descriptor.control;
    uint32_t length = (control >> 14) & 0x3fffu;
    if ((control & (1u << 31)) != 0 || (control & (1u << 30)) == 0 ||
        length < WIFI_RX_METADATA + 4u || length > WIFI_RX_CAPACITY) {
        return C6_MAC_INVALID_RX_DESCRIPTOR;
    }
    *wire_length = length - WIFI_RX_METADATA;
    *rssi = (int8_t)c6_rf_probe_rx_buffer[11];
    c6_write32(C6_REG_WIFI_INTERRUPT_CLEAR, WIFI_RX_DONE);
    c6_mac_rx_start();
    return C6_MAC_RX_FRAME;
}

int c6_mac_rx_copy(uint8_t *frame, uint32_t capacity,
                   uint32_t *length, int8_t *rssi)
{
    if ((c6_read32(C6_REG_WIFI_INTERRUPT_EVENT) & WIFI_RX_DONE) == 0) {
        return C6_MAC_NO_PACKET;
    }
    uint32_t control = c6_rf_probe_rx_descriptor.control;
    uint32_t dma_length = (control >> 14) & 0x3fffu;
    if ((control & (1u << 31)) != 0 || (control & (1u << 30)) == 0 ||
        dma_length < WIFI_RX_METADATA + 4u || dma_length > WIFI_RX_CAPACITY) {
        return C6_MAC_INVALID_RX_DESCRIPTOR;
    }
    uint32_t frame_length = dma_length - WIFI_RX_METADATA - 4u;
    if (frame_length > capacity) return C6_MAC_BUFFER_TOO_SMALL;
    for (uint32_t index = 0; index < frame_length; ++index) {
        frame[index] = c6_rf_probe_rx_buffer[WIFI_RX_METADATA + index];
    }
    *length = frame_length;
    *rssi = (int8_t)c6_rf_probe_rx_buffer[11];
    c6_write32(C6_REG_WIFI_INTERRUPT_CLEAR, WIFI_RX_DONE);
    c6_mac_rx_start();
    return C6_MAC_RX_FRAME;
}

void c6_mac_set_interface_address(const uint8_t address[6])
{
    uint32_t low = (uint32_t)address[0] | (uint32_t)address[1] << 8 |
                   (uint32_t)address[2] << 16 | (uint32_t)address[3] << 24;
    uint32_t high = (uint32_t)address[4] | (uint32_t)address[5] << 8 |
                    WIFI_INTERFACE_ADDRESS_VALID;
    c6_write32(C6_REG_WIFI_INTERFACE0_LOW, low);
    c6_write32(C6_REG_WIFI_INTERFACE0_HIGH, high);
}

static uint32_t load_le32(const uint8_t value[4])
{
    return (uint32_t)value[0] | (uint32_t)value[1] << 8 |
           (uint32_t)value[2] << 16 | (uint32_t)value[3] << 24;
}

void c6_mac_install_ccmp(const uint8_t peer[6], const uint8_t key[16])
{
    uint32_t match = (uint32_t)peer[0] | (uint32_t)peer[1] << 8 |
                     (uint32_t)peer[2] << 16 | (uint32_t)peer[3] << 24;
    uint32_t control = (uint32_t)peer[4] | (uint32_t)peer[5] << 8 |
                       (3u << 18) | (3u << 21);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0, match);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 4u, control);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 8u, load_le32(key));
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 12u, load_le32(key + 4u));
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 16u, load_le32(key + 8u));
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 20u, load_le32(key + 12u));
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 24u, 0);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 28u, 0);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 32u, 0);
    c6_write32(C6_REG_WIFI_CRYPTO_SLOT0 + 36u, 0);
    c6_write32(C6_REG_WIFI_CRYPTO_VALID,
               c6_read32(C6_REG_WIFI_CRYPTO_VALID) | 1u);
}
