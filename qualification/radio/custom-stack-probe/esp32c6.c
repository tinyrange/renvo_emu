/* SPDX-License-Identifier: Apache-2.0 */

#include <stdint.h>

#define READ32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define WRITE32(address, value) (READ32(address) = (uint32_t)(value))

/*
 * Original low-level firmware. It deliberately includes no ESP-IDF headers
 * and links no Espressif runtime or radio library. The addresses and command
 * encodings are the native hardware contract recorded by RADIO_PLAN.html.
 */
#define WIFI_MAC_BASE 0x600a4000u
#define WIFI_INTERRUPT_MASK (WIFI_MAC_BASE + 0x0c40u)
#define WIFI_INTERRUPT_EVENT (WIFI_MAC_BASE + 0x0c48u)
#define WIFI_INTERRUPT_CLEAR (WIFI_MAC_BASE + 0x0c4cu)
#define WIFI_RX_BASE (WIFI_MAC_BASE + 0x0084u)
#define WIFI_QUEUE_STATE_CLEAR (WIFI_MAC_BASE + 0x0cb4u)
#define WIFI_QUEUE_STATE (WIFI_MAC_BASE + 0x0cb8u)
#define WIFI_QUEUE2_CONTROL (WIFI_MAC_BASE + 0x0d6cu)
#define WIFI_RESET_CONTROL (WIFI_MAC_BASE + 0x0ddcu)
#define WIFI_TX_DONE (1u << 7)
#define WIFI_RX_DONE (1u << 14)
#define WIFI_QUEUE_ENABLE (3u << 30)

#define INTERRUPT_MATRIX_BASE 0x60010000u
#define INTERRUPT_MATRIX_WIFI_ROUTE (INTERRUPT_MATRIX_BASE + 0x0000u)
#define INTERRUPT_MATRIX_STATUS0 (INTERRUPT_MATRIX_BASE + 0x0134u)
#define WIFI_CPU_INTERRUPT 5u
#define BLE_CPU_INTERRUPT 8u
#define INTERRUPT_MATRIX_BLE_BB_ROUTE (INTERRUPT_MATRIX_BASE + 5u * 4u)
#define PLIC_MACHINE_BASE 0x20001000u
#define PLIC_ENABLE (PLIC_MACHINE_BASE + 0x0000u)
#define PLIC_PENDING (PLIC_MACHINE_BASE + 0x000cu)
#define PLIC_PRIORITY(line) (PLIC_MACHINE_BASE + 0x0010u + (line) * 4u)
#define PLIC_THRESHOLD (PLIC_MACHINE_BASE + 0x0090u)

#define IEEE802154_BASE 0x600a3000u
#define IEEE802154_COMMAND (IEEE802154_BASE + 0x0000u)
#define IEEE802154_CONFIGURATION (IEEE802154_BASE + 0x0004u)
#define IEEE802154_PAN0_SHORT_ADDRESS (IEEE802154_BASE + 0x0008u)
#define IEEE802154_PAN0_ID (IEEE802154_BASE + 0x000cu)
#define IEEE802154_PAN1_SHORT_ADDRESS (IEEE802154_BASE + 0x0018u)
#define IEEE802154_PAN1_ID (IEEE802154_BASE + 0x001cu)
#define IEEE802154_CHANNEL (IEEE802154_BASE + 0x0048u)
#define IEEE802154_ED_DURATION (IEEE802154_BASE + 0x0050u)
#define IEEE802154_ED_CONFIGURATION (IEEE802154_BASE + 0x0054u)
#define IEEE802154_EVENT_ENABLE (IEEE802154_BASE + 0x0060u)
#define IEEE802154_EVENT_STATUS (IEEE802154_BASE + 0x0064u)
#define IEEE802154_RX_STATUS (IEEE802154_BASE + 0x0080u)
#define IEEE802154_TX_STATUS (IEEE802154_BASE + 0x0084u)
#define IEEE802154_TIMER0_THRESHOLD (IEEE802154_BASE + 0x00a8u)
#define IEEE802154_TX_DMA_ADDRESS (IEEE802154_BASE + 0x00d0u)
#define IEEE802154_RX_DMA_ADDRESS (IEEE802154_BASE + 0x00e0u)
#define IEEE802154_SECURITY_CONTROL (IEEE802154_BASE + 0x0128u)
#define IEEE802154_DEBUG_TX_SECURITY_ERRORS (IEEE802154_BASE + 0x0178u)
#define IEEE802154_MAC_DATE (IEEE802154_BASE + 0x0184u)
#define IEEE802154_TX_DONE (1u << 0)
#define IEEE802154_RX_DONE (1u << 1)
#define IEEE802154_ACK_TX_DONE (1u << 2)
#define IEEE802154_ACK_RX_DONE (1u << 3)
#define IEEE802154_RX_ABORT (1u << 4)
#define IEEE802154_TX_ABORT (1u << 5)
#define IEEE802154_ED_DONE (1u << 6)
#define IEEE802154_TIMER0_DONE (1u << 8)
#define IEEE802154_TX_START 0x41u
#define IEEE802154_RX_START 0x42u
#define IEEE802154_CCA_TX_START 0x43u
#define IEEE802154_ED_START 0x44u
#define IEEE802154_STOP 0x45u
#define IEEE802154_TIMER0_START 0x4cu
#define IEEE802154_TIMER0_STOP 0x4du
#define IEEE802154_AUTOMATIC_ACK_TRANSMIT (1u << 0)
#define IEEE802154_AUTOMATIC_ACK_RECEIVE (1u << 3)
#define IEEE802154_PROMISCUOUS (1u << 7)
#define IEEE802154_PAN0_ENABLE (1u << 28)
#define IEEE802154_PAN1_ENABLE (1u << 29)

#define BLE_BASEBAND_BASE 0x600a1000u
#define BLE_BASEBAND_SCHEDULER_KICK (BLE_BASEBAND_BASE + 0x028u)
#define BLE_BASEBAND_INTERRUPT_ENABLE0 (BLE_BASEBAND_BASE + 0x304u)
#define BLE_BASEBAND_INTERRUPT_CLEAR0 (BLE_BASEBAND_BASE + 0x308u)
#define BLE_BASEBAND_INTERRUPT_RAW0 (BLE_BASEBAND_BASE + 0x30cu)
#define BLE_BASEBAND_SCHEDULER_HEAD (BLE_BASEBAND_BASE + 0x8fcu)
#define BLE_BASEBAND_TIMER_CURRENT (BLE_BASEBAND_BASE + 0x924u)
#define BLE_BASEBAND_RESET (BLE_BASEBAND_BASE + 0xff0u)
#define BLE_EVENT_END (1u << 21)
#define BLE_EVENT_RX (1u << 27)
#define BLE_EVENT_SUCCESS (1u << 28)
#define BLE_SCHEDULE_OWNED (1u << 13)

#define MODEM_SYSCON_CLOCK_CONFIG 0x600a9804u
#define MODEM_SYSCON_WIFI_CLOCK_ENABLE 0x600a9814u
#define MODEM_WIFI_APB_CLOCK (1u << 9)
#define MODEM_WIFI_MAC_CLOCK (1u << 10)
#define MODEM_BLE_APB_CLOCK (1u << 17)
#define MODEM_BLE_BB_CLOCK (1u << 18)
#define MODEM_ZB_APB_CLOCK (1u << 23)
#define MODEM_ZB_MAC_CLOCK (1u << 24)
#define MODEM_SECURITY_CCM_CLOCK (1u << 26)

static uint8_t ieee802154_frame[] = {4, 0x01, 0x00, 0x2a, 0xa5};
static uint8_t ieee802154_ack_frame[] = {4, 0x21, 0x00, 0x44, 0xa5};
static uint8_t ieee802154_no_ack_frame[] = {4, 0x21, 0x00, 0x45, 0xa5};
static uint8_t ieee802154_invalid_security_frame[] = {4, 0x01, 0x00, 0x2b, 0xa5};
uint8_t remu_ieee802154_rx_buffer[128] __attribute__((aligned(4)));
uint8_t remu_wifi_frame[] __attribute__((aligned(4))) = {
    /* Native TX wire length: 24 guest MAC bytes plus generated FCS. */
    28, 0, 0, 0, 0, 0, 0, 0,
    0x40, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x02, 0x00, 0x00, 0x00, 0x00, 0x06,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x00, 0x00,
};
volatile uint32_t remu_wifi_descriptor[3] __attribute__((aligned(4)));
uint8_t remu_wifi_rx_buffer[512] __attribute__((aligned(4)));
volatile uint32_t remu_wifi_rx_descriptor[3] __attribute__((aligned(4)));

uint8_t remu_ble_tx_schedule[64] __attribute__((aligned(4)));
uint8_t remu_ble_tx_state[128] __attribute__((aligned(4)));
uint8_t remu_ble_tx_header[16] __attribute__((aligned(4)));
uint8_t remu_ble_tx_buffer[64] __attribute__((aligned(4)));
uint8_t remu_ble_rx_schedule[64] __attribute__((aligned(4)));
uint8_t remu_ble_rx_state[128] __attribute__((aligned(4)));
uint8_t remu_ble_rx_descriptor[16] __attribute__((aligned(4)));
uint8_t remu_ble_rx_payload[128] __attribute__((aligned(4)));

static void write32(uint8_t *bytes, uint32_t offset, uint32_t value)
{
    bytes[offset] = (uint8_t)value;
    bytes[offset + 1u] = (uint8_t)(value >> 8);
    bytes[offset + 2u] = (uint8_t)(value >> 16);
    bytes[offset + 3u] = (uint8_t)(value >> 24);
}

static uint32_t pointer20(const void *address)
{
    return (uint32_t)(uintptr_t)address & 0x000fffffu;
}

static void dma_publish(void)
{
    __asm__ volatile("fence rw, rw" ::: "memory");
}

static int verify_wifi_rx(void)
{
    const uint32_t capacity = sizeof(remu_wifi_rx_buffer);
    remu_wifi_rx_descriptor[0] =
        (1u << 31) | (capacity << 14) | capacity;
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
        return 10;
    }
    uint32_t control = remu_wifi_rx_descriptor[0];
    uint32_t length = (control >> 14) & 0x3fffu;
    if ((control & (1u << 31)) != 0 || (control & (1u << 30)) == 0 ||
        length < 92u + 24u + 4u) {
        return 11;
    }
    const uint8_t *frame = remu_wifi_rx_buffer + 92;
    if (frame[0] != 0x80 || frame[1] != 0x00 ||
        frame[4] != 0xff || frame[9] != 0xff ||
        frame[10] != 0x02 || frame[15] != 0x01) {
        return 12;
    }
    WRITE32(WIFI_INTERRUPT_CLEAR, WIFI_RX_DONE);
    if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_RX_DONE) != 0) {
        return 13;
    }
    return 0;
}

static int verify_ble(void)
{
    static const uint8_t advertising_data[] = {
        0x02, 0x01, 0x06, 0x0b, 0x09,
        'R', 'e', 'n', 'v', 'o', '-', 'B', 'L', 'E', '1',
    };
    static const uint8_t expected_rx[] = {
        0x42, 0x0c, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xc1, 0x02, 0x01, 0x06, 0x02, 0x09, 0x52,
    };

    WRITE32(INTERRUPT_MATRIX_BLE_BB_ROUTE, BLE_CPU_INTERRUPT);
    WRITE32(PLIC_ENABLE, READ32(PLIC_ENABLE) | (1u << BLE_CPU_INTERRUPT));
    WRITE32(PLIC_PRIORITY(BLE_CPU_INTERRUPT), 3u);
    WRITE32(MODEM_SYSCON_WIFI_CLOCK_ENABLE,
            READ32(MODEM_SYSCON_WIFI_CLOCK_ENABLE) |
                MODEM_BLE_APB_CLOCK | MODEM_BLE_BB_CLOCK);
    WRITE32(BLE_BASEBAND_RESET, 0u);
    WRITE32(BLE_BASEBAND_RESET, 1u);
    WRITE32(BLE_BASEBAND_INTERRUPT_ENABLE0,
            BLE_EVENT_END | BLE_EVENT_RX | BLE_EVENT_SUCCESS);

    /* One native legacy-advertising schedule, allocator header and PDU. */
    write32(remu_ble_tx_header, 8u, (uintptr_t)remu_ble_tx_buffer);
    remu_ble_tx_buffer[0x10] = 0x06;
    remu_ble_tx_buffer[0x11] = 6u + sizeof(advertising_data);
    for (uint32_t index = 0; index < sizeof(advertising_data); ++index) {
        remu_ble_tx_buffer[0x12u + index] = advertising_data[index];
    }
    remu_ble_tx_state[0x34] = 0x02;
    remu_ble_tx_state[0x39] = 0xc6;
    write32(remu_ble_tx_state, 0x60u, (uintptr_t)remu_ble_tx_header);
    write32(remu_ble_tx_schedule, 4u, (uintptr_t)remu_ble_tx_state);
    uint32_t start = READ32(BLE_BASEBAND_TIMER_CURRENT) + 2u;
    write32(remu_ble_tx_schedule, 8u, start);
    write32(remu_ble_tx_schedule, 0x0cu, start + 200u);
    write32(remu_ble_tx_schedule, 0x28u, BLE_SCHEDULE_OWNED);
    remu_ble_tx_schedule[0x35] = 1u;
    dma_publish();
    WRITE32(BLE_BASEBAND_SCHEDULER_HEAD, pointer20(remu_ble_tx_schedule));
    WRITE32(BLE_BASEBAND_SCHEDULER_KICK, 1u);

    uint32_t event = 0;
    for (uint32_t timeout = 0; timeout < 50000u; ++timeout) {
        event = READ32(BLE_BASEBAND_INTERRUPT_RAW0);
        if ((event & BLE_EVENT_END) != 0) {
            break;
        }
    }
    if ((event & (BLE_EVENT_END | BLE_EVENT_SUCCESS)) !=
            (BLE_EVENT_END | BLE_EVENT_SUCCESS) ||
        (*(volatile uint32_t *)(void *)(remu_ble_tx_schedule + 0x28u) &
         BLE_SCHEDULE_OWNED) != 0) {
        return 16;
    }
    if ((READ32(INTERRUPT_MATRIX_STATUS0) & (1u << 5)) == 0 ||
        (READ32(PLIC_PENDING) & (1u << BLE_CPU_INTERRUPT)) == 0) {
        return 17;
    }
    WRITE32(BLE_BASEBAND_INTERRUPT_CLEAR0,
            BLE_EVENT_END | BLE_EVENT_SUCCESS);
    if ((READ32(BLE_BASEBAND_INTERRUPT_RAW0) &
         (BLE_EVENT_END | BLE_EVENT_SUCCESS)) != 0) {
        return 18;
    }

    /* A firmware-owned native RX ring and scan schedule. */
    write32(remu_ble_rx_descriptor, 4u,
            (uintptr_t)(remu_ble_rx_descriptor + 4u));
    write32(remu_ble_rx_descriptor, 8u, (uintptr_t)remu_ble_rx_payload);
    write32(remu_ble_rx_payload, 0x18u, 0xffffu);
    /* CURRENT_RX uses the native header-plus-four cursor convention. */
    write32(remu_ble_rx_state, 8u,
            (uintptr_t)(remu_ble_rx_descriptor + 4u));
    write32(remu_ble_rx_state, 0x2cu, 50000u);
    write32(remu_ble_rx_state, 0x5cu, (uintptr_t)remu_ble_rx_descriptor);
    write32(remu_ble_rx_schedule, 4u, (uintptr_t)remu_ble_rx_state);
    start = READ32(BLE_BASEBAND_TIMER_CURRENT) + 2u;
    write32(remu_ble_rx_schedule, 8u, start);
    write32(remu_ble_rx_schedule, 0x0cu, start + 200u);
    write32(remu_ble_rx_schedule, 0x28u, BLE_SCHEDULE_OWNED);
    remu_ble_rx_schedule[0x35] = 2u;
    dma_publish();
    WRITE32(BLE_BASEBAND_SCHEDULER_HEAD, pointer20(remu_ble_rx_schedule));
    WRITE32(BLE_BASEBAND_SCHEDULER_KICK, 1u);

    event = 0;
    for (uint32_t timeout = 0; timeout < 60000u; ++timeout) {
        event = READ32(BLE_BASEBAND_INTERRUPT_RAW0);
        if ((event & BLE_EVENT_RX) != 0) {
            break;
        }
    }
    if ((event & (BLE_EVENT_END | BLE_EVENT_RX | BLE_EVENT_SUCCESS)) !=
        (BLE_EVENT_END | BLE_EVENT_RX | BLE_EVENT_SUCCESS)) {
        return 19;
    }
    for (uint32_t index = 0; index < sizeof(expected_rx); ++index) {
        if (remu_ble_rx_payload[0x1cu + index] != expected_rx[index]) {
            return 20;
        }
    }
    if (remu_ble_rx_payload[0x0fu] != (uint8_t)-80 ||
        (*(volatile uint32_t *)(void *)(remu_ble_rx_state + 0x14u) &
         BLE_EVENT_RX) == 0) {
        return 21;
    }
    WRITE32(BLE_BASEBAND_INTERRUPT_CLEAR0,
            BLE_EVENT_END | BLE_EVENT_RX | BLE_EVENT_SUCCESS);
    if ((READ32(BLE_BASEBAND_INTERRUPT_RAW0) &
         (BLE_EVENT_END | BLE_EVENT_RX | BLE_EVENT_SUCCESS)) != 0 ||
        (READ32(INTERRUPT_MATRIX_STATUS0) & (1u << 5)) != 0 ||
        (READ32(PLIC_PENDING) & (1u << BLE_CPU_INTERRUPT)) != 0) {
        return 22;
    }
    return 0;
}

static int verify_ieee802154_filter_and_ack(void)
{
    uint32_t event = 0;

    /* Two native filter interfaces, matching the independent PAN1 fixture. */
    WRITE32(IEEE802154_PAN0_ID, 0x1234u);
    WRITE32(IEEE802154_PAN0_SHORT_ADDRESS, 0x5678u);
    WRITE32(IEEE802154_PAN1_ID, 0xabcdu);
    WRITE32(IEEE802154_PAN1_SHORT_ADDRESS, 0x1357u);
    WRITE32(IEEE802154_CONFIGURATION,
            IEEE802154_AUTOMATIC_ACK_RECEIVE |
                IEEE802154_PAN0_ENABLE | IEEE802154_PAN1_ENABLE);
    WRITE32(IEEE802154_RX_DMA_ADDRESS,
            (uintptr_t)remu_ieee802154_rx_buffer);
    WRITE32(IEEE802154_COMMAND, IEEE802154_RX_START);

    /* The first addressed frame targets neither enabled interface. */
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_RX_ABORT | IEEE802154_RX_DONE)) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_RX_ABORT) == 0 ||
        (event & IEEE802154_RX_DONE) != 0 ||
        READ32(IEEE802154_RX_STATUS) != 0x51u) {
        return 23;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_RX_ABORT);

    /* A filter abort ends one-shot RX. Re-arm for PAN1's matching frame. */
    WRITE32(IEEE802154_COMMAND, IEEE802154_RX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_RX_ABORT | IEEE802154_RX_DONE)) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_RX_DONE) == 0 ||
        remu_ieee802154_rx_buffer[0] != 11u ||
        remu_ieee802154_rx_buffer[1] != 0x01u ||
        remu_ieee802154_rx_buffer[2] != 0x08u ||
        remu_ieee802154_rx_buffer[3] != 0x32u ||
        remu_ieee802154_rx_buffer[4] != 0xcdu ||
        remu_ieee802154_rx_buffer[5] != 0xabu ||
        remu_ieee802154_rx_buffer[6] != 0x57u ||
        remu_ieee802154_rx_buffer[7] != 0x13u ||
        remu_ieee802154_rx_buffer[10] != (uint8_t)-80 ||
        remu_ieee802154_rx_buffer[11] != 63u) {
        return 24;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_RX_DONE);

    /* ACK-request TX enters native RX_ACK. Timer0 is owned by this stack. */
    WRITE32(IEEE802154_TX_DMA_ADDRESS,
            (uintptr_t)ieee802154_ack_frame);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & IEEE802154_TX_DONE) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TX_DONE) == 0) {
        return 25;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TX_DONE);
    WRITE32(IEEE802154_TIMER0_THRESHOLD, 10000u);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TIMER0_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_ACK_RX_DONE | IEEE802154_TIMER0_DONE)) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_ACK_RX_DONE) == 0 ||
        (event & IEEE802154_TIMER0_DONE) != 0 ||
        remu_ieee802154_rx_buffer[0] != 5u ||
        remu_ieee802154_rx_buffer[1] != 0x02u ||
        remu_ieee802154_rx_buffer[2] != 0x00u ||
        remu_ieee802154_rx_buffer[3] != 0x44u ||
        remu_ieee802154_rx_buffer[4] != (uint8_t)-80 ||
        remu_ieee802154_rx_buffer[5] != 63u) {
        return 26;
    }
    WRITE32(IEEE802154_COMMAND, IEEE802154_TIMER0_STOP);
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_ACK_RX_DONE);

    /* A second ACK-request has no injected ACK and must expire on Timer0. */
    WRITE32(IEEE802154_TX_DMA_ADDRESS,
            (uintptr_t)ieee802154_no_ack_frame);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & IEEE802154_TX_DONE) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TX_DONE) == 0) {
        return 27;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TX_DONE);
    WRITE32(IEEE802154_TIMER0_THRESHOLD, 512u);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TIMER0_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_ACK_RX_DONE | IEEE802154_TIMER0_DONE)) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TIMER0_DONE) == 0 ||
        (event & IEEE802154_ACK_RX_DONE) != 0) {
        return 28;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TIMER0_DONE);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TIMER0_STOP);
    WRITE32(IEEE802154_COMMAND, IEEE802154_STOP);

    /* A matching unicast requesting an ACK must complete RX and auto-TX ACK. */
    WRITE32(IEEE802154_CONFIGURATION,
            IEEE802154_AUTOMATIC_ACK_TRANSMIT |
                IEEE802154_AUTOMATIC_ACK_RECEIVE |
                IEEE802154_PAN0_ENABLE | IEEE802154_PAN1_ENABLE);
    WRITE32(IEEE802154_COMMAND, IEEE802154_RX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_RX_DONE | IEEE802154_ACK_TX_DONE)) ==
            (IEEE802154_RX_DONE | IEEE802154_ACK_TX_DONE)) {
            break;
        }
    }
    if ((event & (IEEE802154_RX_DONE | IEEE802154_ACK_TX_DONE)) !=
            (IEEE802154_RX_DONE | IEEE802154_ACK_TX_DONE) ||
        remu_ieee802154_rx_buffer[0] != 11u ||
        remu_ieee802154_rx_buffer[1] != 0x21u ||
        remu_ieee802154_rx_buffer[2] != 0x08u ||
        remu_ieee802154_rx_buffer[3] != 0x46u ||
        remu_ieee802154_rx_buffer[4] != 0xcdu ||
        remu_ieee802154_rx_buffer[5] != 0xabu ||
        remu_ieee802154_rx_buffer[10] != (uint8_t)-80 ||
        remu_ieee802154_rx_buffer[11] != 63u) {
        return 29;
    }
    WRITE32(IEEE802154_EVENT_STATUS,
            IEEE802154_RX_DONE | IEEE802154_ACK_TX_DONE);

    /* C6 CCM is a transmit peripheral. Preserve its published negative
     * reason when security is requested without the FCF security bit. */
    WRITE32(IEEE802154_SECURITY_CONTROL, 1u | (5u << 8));
    WRITE32(IEEE802154_TX_DMA_ADDRESS,
            (uintptr_t)ieee802154_invalid_security_frame);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & IEEE802154_TX_ABORT) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TX_ABORT) == 0 ||
        READ32(IEEE802154_TX_STATUS) != ((19u << 4) | (1u << 16)) ||
        READ32(IEEE802154_DEBUG_TX_SECURITY_ERRORS) != 1u) {
        return 30;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TX_ABORT);
    WRITE32(IEEE802154_SECURITY_CONTROL, 0u);

    /* CCA_TX_START performs one eight-symbol hardware assessment. Guest
     * firmware owns any CSMA-CA retry policy after a busy result. */
    WRITE32(IEEE802154_ED_DURATION, 8u);
    WRITE32(IEEE802154_ED_CONFIGURATION,
            (uint32_t)(uint8_t)-75 | (1u << 14));
    WRITE32(IEEE802154_TX_DMA_ADDRESS, (uintptr_t)ieee802154_frame);
    WRITE32(IEEE802154_COMMAND, IEEE802154_CCA_TX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_TX_DONE | IEEE802154_TX_ABORT)) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TX_DONE) == 0 ||
        (event & IEEE802154_TX_ABORT) != 0) {
        return 31;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TX_DONE);
    return 0;
}

int main(void)
{
    WRITE32(WIFI_RESET_CONTROL, 1u << 1);
    if ((READ32(WIFI_RESET_CONTROL) & 1u) == 0) {
        return 1;
    }

    WRITE32(INTERRUPT_MATRIX_WIFI_ROUTE, WIFI_CPU_INTERRUPT);
    WRITE32(PLIC_ENABLE, 1u << WIFI_CPU_INTERRUPT);
    WRITE32(PLIC_PRIORITY(WIFI_CPU_INTERRUPT), 3u);
    WRITE32(PLIC_THRESHOLD, 1u);
    WRITE32(MODEM_SYSCON_WIFI_CLOCK_ENABLE,
            READ32(MODEM_SYSCON_WIFI_CLOCK_ENABLE) |
                MODEM_WIFI_APB_CLOCK | MODEM_WIFI_MAC_CLOCK |
                MODEM_BLE_APB_CLOCK | MODEM_BLE_BB_CLOCK);
    WRITE32(WIFI_INTERRUPT_MASK, WIFI_TX_DONE | WIFI_RX_DONE);
    remu_wifi_descriptor[0] = 0xc0000000u;
    remu_wifi_descriptor[1] = (uintptr_t)remu_wifi_frame;
    remu_wifi_descriptor[2] = 0;
    WRITE32(WIFI_QUEUE2_CONTROL,
            WIFI_QUEUE_ENABLE |
                ((uintptr_t)remu_wifi_descriptor & 0x000fffffu));
    if ((READ32(WIFI_QUEUE_STATE) & 1u) == 0) {
        return 2;
    }
    if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_TX_DONE) == 0) {
        return 3;
    }
    if ((READ32(INTERRUPT_MATRIX_STATUS0) & 1u) == 0 ||
        (READ32(PLIC_PENDING) & (1u << WIFI_CPU_INTERRUPT)) == 0) {
        return 4;
    }
    WRITE32(WIFI_INTERRUPT_CLEAR, WIFI_TX_DONE);
    WRITE32(WIFI_QUEUE_STATE_CLEAR, 1u);
    if ((READ32(WIFI_INTERRUPT_EVENT) & WIFI_TX_DONE) != 0 ||
        (READ32(WIFI_QUEUE_STATE) & 1u) != 0 ||
        (READ32(INTERRUPT_MATRIX_STATUS0) & 1u) != 0 ||
        (READ32(PLIC_PENDING) & (1u << WIFI_CPU_INTERRUPT)) != 0) {
        return 5;
    }

    int wifi_rx_failure = verify_wifi_rx();
    if (wifi_rx_failure != 0) {
        return wifi_rx_failure;
    }

    if ((READ32(IEEE802154_MAC_DATE) & 0x0fffffffu) != 0x00220622u) {
        return 6;
    }
    WRITE32(MODEM_SYSCON_CLOCK_CONFIG,
            MODEM_ZB_APB_CLOCK | MODEM_ZB_MAC_CLOCK |
                MODEM_SECURITY_CCM_CLOCK);
    /* Native HOP/frequency encoding: channel 11 is frequency code 3. */
    WRITE32(IEEE802154_CHANNEL, 3u);
    WRITE32(IEEE802154_EVENT_ENABLE, 0x1fffu);
    WRITE32(IEEE802154_TX_DMA_ADDRESS, (uintptr_t)ieee802154_frame);
    WRITE32(IEEE802154_COMMAND, IEEE802154_TX_START);
    uint32_t event = 0;
    for (uint32_t timeout = 0; timeout < 1000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_TX_DONE | (1u << 5))) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_TX_DONE) == 0) {
        return 7;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_TX_DONE);
    if ((READ32(IEEE802154_EVENT_STATUS) & IEEE802154_TX_DONE) != 0) {
        return 8;
    }

    WRITE32(IEEE802154_CONFIGURATION, IEEE802154_PROMISCUOUS);
    WRITE32(IEEE802154_RX_DMA_ADDRESS,
            (uintptr_t)remu_ieee802154_rx_buffer);
    WRITE32(IEEE802154_COMMAND, IEEE802154_RX_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 20000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & (IEEE802154_RX_DONE | (1u << 4))) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_RX_DONE) == 0 ||
        remu_ieee802154_rx_buffer[0] != 6 ||
        remu_ieee802154_rx_buffer[1] != 0x01 ||
        remu_ieee802154_rx_buffer[2] != 0x00 ||
        remu_ieee802154_rx_buffer[3] != 0x02 ||
        remu_ieee802154_rx_buffer[4] != 0xaa ||
        remu_ieee802154_rx_buffer[5] != (uint8_t)-80 ||
        remu_ieee802154_rx_buffer[6] != 63) {
        return 14;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_RX_DONE);
    if ((READ32(IEEE802154_EVENT_STATUS) & IEEE802154_RX_DONE) != 0) {
        return 15;
    }

    WRITE32(IEEE802154_COMMAND, IEEE802154_ED_START);
    event = 0;
    for (uint32_t timeout = 0; timeout < 1000u; ++timeout) {
        event = READ32(IEEE802154_EVENT_STATUS);
        if ((event & IEEE802154_ED_DONE) != 0) {
            break;
        }
    }
    if ((event & IEEE802154_ED_DONE) == 0) {
        return 9;
    }
    WRITE32(IEEE802154_EVENT_STATUS, IEEE802154_ED_DONE);

    int ieee802154_failure = verify_ieee802154_filter_and_ack();
    if (ieee802154_failure != 0) {
        return ieee802154_failure;
    }

    int ble_failure = verify_ble();
    if (ble_failure != 0) {
        return ble_failure;
    }
    return 0;
}
