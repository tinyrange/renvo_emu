/* SPDX-License-Identifier: Apache-2.0 */

#include <stdint.h>

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

/*
 * Original low-level firmware. It deliberately includes no ESP-IDF headers
 * and links no Espressif runtime or radio library. The addresses and command
 * encodings are the native hardware contract recorded by RADIO_PLAN.html.
 */
#define WIFI_MAC_BASE 0x60033000u
#define WIFI_INTERRUPT_MASK (WIFI_MAC_BASE + 0x0c34u)
#define WIFI_INTERRUPT_EVENT (WIFI_MAC_BASE + 0x0c3cu)
#define WIFI_INTERRUPT_CLEAR (WIFI_MAC_BASE + 0x0c40u)
#define WIFI_RX_BASE (WIFI_MAC_BASE + 0x0088u)
#define WIFI_QUEUE_STATE_CLEAR (WIFI_MAC_BASE + 0x0cacu)
#define WIFI_QUEUE_STATE (WIFI_MAC_BASE + 0x0cb0u)
#define WIFI_QUEUE0_CONTROL (WIFI_MAC_BASE + 0x0d08u)
#define WIFI_RESET_CONTROL (WIFI_MAC_BASE + 0x0d14u)
#define WIFI_CURRENT_TIME (WIFI_MAC_BASE + 0x2000u)
#define WIFI_TSF_LATCH_CONTROL (WIFI_MAC_BASE + 0x200cu)
#define WIFI_TSF_HIGH (WIFI_MAC_BASE + 0x2018u)
#define WIFI_TSF_LOW (WIFI_MAC_BASE + 0x201cu)
#define WIFI_RANDOM_DATA (WIFI_MAC_BASE + 0x207cu)
#define WIFI_TX_DONE (1u << 7)
#define WIFI_RX_DONE (1u << 14)
#define WIFI_QUEUE_ENABLE (3u << 30)

#define INTERRUPT_MATRIX_BASE 0x600c2000u
#define INTERRUPT_MATRIX_WIFI_ROUTE (INTERRUPT_MATRIX_BASE + 0x0000u)
#define INTERRUPT_MATRIX_STATUS0 (INTERRUPT_MATRIX_BASE + 0x018cu)
#define WIFI_CPU_INTERRUPT 5u

#define BLE_BASE 0x60031000u
#define BLE_INTERRUPT_STATUS (BLE_BASE + 0x010u)
#define BLE_INTERRUPT_CLEAR (BLE_BASE + 0x018u)
#define BLE_TIME_LATCH (BLE_BASE + 0x01cu)
#define BLE_SCHEDULER_KICK (BLE_BASE + 0x100u)
#define BLE_MAPPING0 (BLE_BASE + 0x204u)
#define BLE_MAPPING_VALID_LOW (BLE_BASE + 0x2c4u)
#define BLE_RX_INTERRUPT (1u << 2)
#define BLE_END_INTERRUPT (1u << 5)
#define BLE_TIME_LATCH_REQUEST (1u << 31)
#define BLE_SCHEDULER_START (1u << 31)

uint8_t remu_wifi_frame[] __attribute__((aligned(4))) = {
    0x40, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x02, 0x00, 0x00, 0x00, 0x00, 0x03,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00,
};
volatile uint32_t remu_wifi_descriptor[3] __attribute__((aligned(4)));
uint8_t remu_wifi_rx_buffer[512] __attribute__((aligned(4)));
volatile uint32_t remu_wifi_rx_descriptor[3] __attribute__((aligned(4)));

uint8_t remu_ble_slot[16] __attribute__((aligned(4)));
uint8_t remu_ble_control[90] __attribute__((aligned(4)));
uint8_t remu_ble_tx_descriptor[32] __attribute__((aligned(4)));
uint8_t remu_ble_payload[] __attribute__((aligned(4))) = {
    0x02, 0x01, 0x06, 0x0b, 0x09,
    'R', 'e', 'n', 'v', 'o', '-', 'B', 'L', 'E', '1',
};
uint8_t remu_ble_rx_descriptor[20] __attribute__((aligned(4)));
uint8_t remu_ble_rx_payload[64] __attribute__((aligned(4)));

static void write16(uint8_t *bytes, uint32_t offset, uint16_t value)
{
    bytes[offset] = (uint8_t)value;
    bytes[offset + 1u] = (uint8_t)(value >> 8);
}

static void write32(uint8_t *bytes, uint32_t offset, uint32_t value)
{
    write16(bytes, offset, (uint16_t)value);
    write16(bytes, offset + 2u, (uint16_t)(value >> 16));
}

static uint32_t ble_mapping(uint32_t em_offset, const void *cpu_address)
{
    return ((em_offset >> 2) << 18) |
           (((uint32_t)(uintptr_t)cpu_address & 0x000fffffu) >> 2);
}

__attribute__((noreturn, section(".text.start"))) void _start(void)
{
    volatile uint32_t *const exit_code = (volatile uint32_t *)0xfffffff0u;
    uint32_t failure = 0;

    WRITE32(WIFI_RESET_CONTROL, 1u << 1);
    if ((READ32(WIFI_RESET_CONTROL) & 1u) == 0) {
        failure = 1;
    }

    if (failure == 0) {
        WRITE32(INTERRUPT_MATRIX_WIFI_ROUTE, WIFI_CPU_INTERRUPT);
        WRITE32(WIFI_INTERRUPT_MASK, WIFI_TX_DONE | WIFI_RX_DONE);
        remu_wifi_descriptor[0] =
            0xc0000000u | ((uint32_t)sizeof(remu_wifi_frame) << 12) |
            (uint32_t)sizeof(remu_wifi_frame);
        remu_wifi_descriptor[1] = (uintptr_t)remu_wifi_frame;
        remu_wifi_descriptor[2] = 0;
        WRITE32(WIFI_QUEUE0_CONTROL,
                WIFI_QUEUE_ENABLE |
                    ((uintptr_t)remu_wifi_descriptor & 0x000fffffu));
        if ((READ32(WIFI_QUEUE_STATE) & 1u) == 0) {
            failure = 2;
        } else if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_TX_DONE) == 0) {
            failure = 3;
        } else if ((READ32(INTERRUPT_MATRIX_STATUS0) & 1u) == 0) {
            failure = 4;
        }
    }
    if (failure == 0) {
        WRITE32(WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE);
        WRITE32(WIFI_QUEUE_STATE_CLEAR, 1u);
        if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_TX_DONE) != 0 ||
            (READ32(WIFI_QUEUE_STATE) & 1u) != 0 ||
            (READ32(INTERRUPT_MATRIX_STATUS0) & 1u) != 0) {
            failure = 5;
        }
    }

    if (failure == 0) {
        const uint32_t capacity = sizeof(remu_wifi_rx_buffer);
        remu_wifi_rx_descriptor[0] =
            (1u << 31) | (capacity << 12) | capacity;
        remu_wifi_rx_descriptor[1] = (uintptr_t)remu_wifi_rx_buffer;
        remu_wifi_rx_descriptor[2] = 0;
        WRITE32(WIFI_RX_BASE, (uintptr_t)remu_wifi_rx_descriptor);
        uint32_t event = 0;
        for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
            event = READ32(WIFI_INTERRUPT_EVENT);
            if ((event & WIFI_RX_DONE) != 0) {
                break;
            }
        }
        if ((event & WIFI_RX_DONE) == 0) {
            failure = 9;
        } else {
            uint32_t control = remu_wifi_rx_descriptor[0];
            uint32_t length = (control >> 12) & 0x0fffu;
            if ((control & (1u << 31)) != 0 ||
                (control & (1u << 30)) == 0 || length < 48u + 24u + 4u) {
                failure = 10;
            } else {
                const uint8_t *frame = remu_wifi_rx_buffer + 48;
                if (frame[0] != 0x80 || frame[1] != 0x00 ||
                    frame[4] != 0xff || frame[9] != 0xff ||
                    frame[10] != 0x02 || frame[15] != 0x01) {
                    failure = 11;
                }
            }
        }
    }
    if (failure == 0) {
        WRITE32(WIFI_INTERRUPT_CLEAR, WIFI_RX_DONE);
        if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_RX_DONE) != 0) {
            failure = 12;
        }
    }

    if (failure == 0) {
        WRITE32(BLE_MAPPING0 + 0u, ble_mapping(0x0000u, remu_ble_slot));
        WRITE32(BLE_MAPPING0 + 4u, ble_mapping(0x0400u, remu_ble_control));
        WRITE32(BLE_MAPPING0 + 8u,
                ble_mapping(0x1400u, remu_ble_tx_descriptor));
        WRITE32(BLE_MAPPING0 + 12u, ble_mapping(0x2400u, remu_ble_payload));
        WRITE32(BLE_MAPPING0 + 16u,
                ble_mapping(0x1000u, remu_ble_rx_descriptor));
        WRITE32(BLE_MAPPING0 + 20u,
                ble_mapping(0x3000u, remu_ble_rx_payload));
        WRITE32(BLE_MAPPING_VALID_LOW, 0x3fu);

        WRITE32(BLE_TIME_LATCH, BLE_TIME_LATCH_REQUEST);
        uint32_t coarse = READ32(BLE_TIME_LATCH) + 2u;
        write16(remu_ble_slot, 0u, 0x2802u);
        write32(remu_ble_slot, 2u, coarse);
        write16(remu_ble_slot, 6u, 624u);
        write16(remu_ble_slot, 8u, 0x0200u);
        write32(remu_ble_control, 12u, 0x8e89bed6u);
        write16(remu_ble_control, 22u, 39u);
        write16(remu_ble_control, 28u, 0x1400u);
        write16(remu_ble_tx_descriptor, 2u, 0x1546u);
        write16(remu_ble_tx_descriptor, 4u, 0x2400u);
        WRITE32(BLE_SCHEDULER_KICK, BLE_SCHEDULER_START);

        uint32_t event = 0;
        for (uint32_t timeout = 0; timeout < 50000u; ++timeout) {
            event = READ32(BLE_INTERRUPT_STATUS);
            if ((event & BLE_END_INTERRUPT) != 0) {
                break;
            }
        }
        if ((event & BLE_END_INTERRUPT) == 0) {
            failure = 13;
        } else if ((remu_ble_slot[0] & 0x38u) != (4u << 3)) {
            failure = 14;
        } else {
            WRITE32(BLE_INTERRUPT_CLEAR, BLE_END_INTERRUPT);
            if ((READ32(BLE_INTERRUPT_STATUS) & BLE_END_INTERRUPT) != 0) {
                failure = 15;
            }
        }
    }

    /*
     * Exercise the same native receive-ring contract used by the vendor
     * controller. The descriptor is owned by RWBLE while bit 15 at +2 is set;
     * reception returns it to firmware by setting bit 15 on the next pointer,
     * clearing ownership, filling the PDU header/metadata/payload, and raising
     * the RX cause. No vendor headers, runtime, libraries, or symbol dispatch
     * participate in this path.
     */
    if (failure == 0) {
        write16(remu_ble_rx_descriptor, 0u, 0x1000u);
        write16(remu_ble_rx_descriptor, 2u, 0x8000u);
        write16(remu_ble_rx_descriptor, 18u, 0x3000u);

        WRITE32(BLE_TIME_LATCH, BLE_TIME_LATCH_REQUEST);
        uint32_t coarse = READ32(BLE_TIME_LATCH) + 2u;
        write16(remu_ble_slot, 0u, 0x0208u);
        write32(remu_ble_slot, 2u, coarse);
        write16(remu_ble_slot, 6u, 624u);
        write16(remu_ble_slot, 8u, 0x0200u);
        write32(remu_ble_control, 12u, 0x8e89bed6u);
        write16(remu_ble_control, 22u, 39u);
        write16(remu_ble_control, 28u, 0u);
        write16(remu_ble_control, 32u, 16u);
        WRITE32(BLE_SCHEDULER_KICK, BLE_SCHEDULER_START);

        uint32_t event = 0;
        for (uint32_t timeout = 0; timeout < 50000u; ++timeout) {
            event = READ32(BLE_INTERRUPT_STATUS);
            if ((event & BLE_RX_INTERRUPT) != 0) {
                break;
            }
        }
        if ((event & BLE_RX_INTERRUPT) == 0) {
            failure = 16;
        } else if ((*(volatile uint16_t *)(void *)(remu_ble_rx_descriptor + 0u) &
                    0x8000u) == 0u ||
                   (*(volatile uint16_t *)(void *)(remu_ble_rx_descriptor + 2u) &
                    0x8000u) != 0u) {
            failure = 17;
        } else if (*(volatile uint16_t *)(void *)(remu_ble_rx_descriptor + 4u) !=
                   0x0c42u) {
            failure = 18;
        } else if (remu_ble_rx_payload[0] != 0xaau ||
                   remu_ble_rx_payload[1] != 0xbbu ||
                   remu_ble_rx_payload[2] != 0xccu ||
                   remu_ble_rx_payload[3] != 0xddu ||
                   remu_ble_rx_payload[4] != 0xeeu ||
                   remu_ble_rx_payload[5] != 0xc1u ||
                   remu_ble_rx_payload[6] != 0x02u ||
                   remu_ble_rx_payload[7] != 0x01u ||
                   remu_ble_rx_payload[8] != 0x06u ||
                   remu_ble_rx_payload[9] != 0x02u ||
                   remu_ble_rx_payload[10] != 0x09u ||
                   remu_ble_rx_payload[11] != 0x52u) {
            failure = 19;
        } else {
            uint16_t rxchass =
                *(volatile uint16_t *)(void *)(remu_ble_rx_descriptor + 14u);
            uint16_t rxrssi =
                *(volatile uint16_t *)(void *)(remu_ble_rx_descriptor + 6u);
            if ((rxchass & 0x3fu) != 39u || (rxrssi & 0xffu) != 0xb0u) {
                failure = 20;
            } else {
                WRITE32(BLE_INTERRUPT_CLEAR, BLE_RX_INTERRUPT);
                if ((READ32(BLE_INTERRUPT_STATUS) & BLE_RX_INTERRUPT) != 0u) {
                    failure = 21;
                }
            }
        }
    }

    if (failure == 0) {
        uint32_t before = READ32(WIFI_CURRENT_TIME);
        uint32_t after = READ32(WIFI_CURRENT_TIME);
        if (after < before) {
            failure = 6;
        }
    }
    if (failure == 0) {
        WRITE32(WIFI_TSF_LATCH_CONTROL, 1u);
        uint32_t low = READ32(WIFI_TSF_LOW);
        uint32_t high = READ32(WIFI_TSF_HIGH);
        if ((low | high) == 0) {
            failure = 7;
        }
    }
    if (failure == 0) {
        uint32_t first = READ32(WIFI_RANDOM_DATA);
        uint32_t second = READ32(WIFI_RANDOM_DATA);
        if (first == 0 || second == 0 || first == second) {
            failure = 8;
        }
    }

    *exit_code = failure;
    __asm__ volatile("break 0, 0");
    for (;;) {
    }
}
