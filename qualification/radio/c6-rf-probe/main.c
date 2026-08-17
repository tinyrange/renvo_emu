/* SPDX-License-Identifier: Apache-2.0 */
#include <stdint.h>

#include "hal/c6/mac.h"
#include "hal/c6/phy.h"
#include "hal/c6/rf.h"
#include "hal/c6/uart.h"
#include "station.h"

static char command[64];
static uint32_t command_length;

static int equals(const char *left, const char *right)
{
    while (*left != '\0' && *right != '\0' && *left == *right) {
        ++left;
        ++right;
    }
    return *left == *right;
}

static int begins(const char *value, const char *prefix)
{
    while (*prefix != '\0') {
        if (*value++ != *prefix++) return 0;
    }
    return 1;
}

static uint32_t parse_u32(const char *value)
{
    uint32_t result = 0;
    while (*value >= '0' && *value <= '9') {
        result = result * 10u + (uint32_t)(*value++ - '0');
    }
    return result;
}

static void checkpoint(const char *event, int result)
{
    c6_uart_puts("REMU_C6_RF event=");
    c6_uart_puts(event);
    c6_uart_puts(" result=");
    if (result < 0) {
        c6_uart_putc('-');
        c6_uart_put_u32((uint32_t)-result);
    } else {
        c6_uart_put_u32((uint32_t)result);
    }
    c6_uart_puts(" channel=");
    c6_uart_put_u32(c6_rf_channel());
    c6_uart_puts(" power_dbm=");
    c6_uart_put_u32(c6_rf_power_dbm());
    c6_uart_putc('\n');
}

static int initialize(void)
{
    int result = c6_phy_init();
    if (result == 0) result = c6_mac_init();
    if (result == 0) result = c6_rf_configure(1, 14);
    checkpoint("INIT", result);
    return result;
}

static void execute(char *line)
{
    int result;
    if (equals(line, "INIT")) {
        (void)initialize();
    } else if (begins(line, "CHANNEL ")) {
        result = c6_rf_set_channel((uint8_t)parse_u32(line + 8));
        checkpoint("CHANNEL", result);
    } else if (begins(line, "POWER ")) {
        result = c6_rf_set_power((uint8_t)parse_u32(line + 6));
        checkpoint("POWER", result);
    } else if (equals(line, "RX START")) {
        c6_mac_rx_start();
        checkpoint("RX_START", 0);
    } else if (equals(line, "RX STOP")) {
        c6_mac_rx_stop();
        checkpoint("RX_STOP", 0);
    } else if (begins(line, "TX ")) {
        result = c6_mac_tx_probe(line + 3);
        checkpoint("TX", result);
    } else if (equals(line, "RESET RADIO")) {
        result = c6_phy_reset_radio();
        checkpoint("RESET_RADIO", result);
    } else {
        checkpoint("BAD_COMMAND", -1);
    }
}

static int stage(uint8_t channel, uint8_t power_dbm, const char *tag)
{
    int result = c6_rf_configure(channel, power_dbm);
    if (result == 0) result = c6_mac_tx_probe(tag);
    checkpoint(tag, result);
    return result;
}

static void self_test(void)
{
    if (initialize() != 0) return;
    if (stage(1, 8, "CH1-P08") != 0) return;
    if (stage(6, 14, "CH6-P14") != 0) return;
    if (stage(11, 20, "CH11-P20") != 0) return;
    if (stage(6, 14, "WARM-P14") != 0) return;
    int result = c6_phy_reset_radio();
    checkpoint("RESET_RADIO", result);
    if (result != 0 || c6_mac_init() != 0) return;
    if (stage(11, 14, "RESET-P14") != 0) return;
    (void)c6_rf_configure(6, 14);
    c6_mac_rx_start();
    checkpoint("READY", 0);
    checkpoint("OPEN_START", c6_station_start());
}

int main(void)
{
    self_test();
    uint8_t received_frame[256];
    for (;;) {
        uint32_t wire_length;
        int8_t rssi;
        int received = c6_mac_rx_copy(received_frame, sizeof(received_frame),
                                       &wire_length, &rssi);
        if (received != 0) {
            c6_uart_puts("REMU_C6_RF event=RX result=");
            c6_uart_put_u32(received > 0 ? 0u : 1u);
            c6_uart_puts(" length=");
            c6_uart_put_u32(received > 0 ? wire_length : 0u);
            c6_uart_puts(" rssi=");
            if (rssi < 0) c6_uart_putc('-');
            c6_uart_put_u32(rssi < 0 ? (uint32_t)-rssi : (uint32_t)rssi);
            c6_uart_putc('\n');
            if (received > 0) {
                uint32_t events = c6_station_receive(received_frame, wire_length);
                if ((events & C6_STATION_SCANNED) != 0) checkpoint("OPEN_SCAN", 0);
                if ((events & C6_STATION_AUTHENTICATED) != 0) checkpoint("OPEN_AUTH", 0);
                if ((events & C6_STATION_ASSOCIATED) != 0) checkpoint("OPEN_ASSOC", 0);
                if ((events & C6_STATION_L2_TX) != 0) checkpoint("OPEN_L2_TX", 0);
                if ((events & C6_STATION_L2_RX) != 0) checkpoint("OPEN_L2_RX", 0);
                if ((events & C6_STATION_FAILED) != 0) checkpoint("OPEN_FAILED", -1);
            }
        }
        int byte = c6_uart_getc_nonblocking();
        if (byte < 0) continue;
        if (byte == '\r') continue;
        if (byte == '\n') {
            command[command_length] = '\0';
            execute(command);
            command_length = 0;
        } else if (command_length + 1u < sizeof(command)) {
            command[command_length++] = (char)byte;
        } else {
            command_length = 0;
            checkpoint("COMMAND_TOO_LONG", -1);
        }
    }
}
