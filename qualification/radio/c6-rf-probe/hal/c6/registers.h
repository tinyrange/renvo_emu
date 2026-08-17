/* SPDX-License-Identifier: Apache-2.0 */
#ifndef REMU_C6_REGISTERS_H
#define REMU_C6_REGISTERS_H

/*
 * Audited MMIO dependency surface for the independent C6 radio driver.
 *
 * Every peripheral address literal must remain in this file and must have an
 * exact entry in ../../mmio-contract.json. check-c6-custom-driver-contract.py
 * rejects undeclared symbols, raw peripheral addresses elsewhere, and access
 * directions that exceed the manifest. Bit fields and ordinary data constants
 * stay beside the code that uses them.
 */

#define C6_REG_RFPLL_CHANNEL_CONTROL 0x600a00c0u
#define C6_REG_TX_GAIN_FIRST 0x600a08ccu
#define C6_REG_TX_GAIN_SECOND 0x600a08d0u
#define C6_REG_TX_GAIN_FINAL 0x600a08d4u
#define C6_REG_RF_FRONTEND_FORCE 0x600a0910u

#define C6_REG_WIFI_INTERFACE0_LOW 0x600a405cu
#define C6_REG_WIFI_INTERFACE0_HIGH 0x600a4060u
#define C6_REG_WIFI_RX_BASE 0x600a4084u
#define C6_REG_WIFI_CRYPTO_VALID 0x600a4814u
#define C6_REG_WIFI_INTERRUPT_MASK 0x600a4c40u
#define C6_REG_WIFI_INTERRUPT_EVENT 0x600a4c48u
#define C6_REG_WIFI_INTERRUPT_CLEAR 0x600a4c4cu
#define C6_REG_WIFI_TX_QUEUE_STATE_CLEAR 0x600a4cb4u
#define C6_REG_WIFI_TX_QUEUE_STATE 0x600a4cb8u
#define C6_REG_WIFI_TX_QUEUE0_CONTROL 0x600a4d6cu
#define C6_REG_WIFI_RESET_CONTROL 0x600a4ddcu
#define C6_REG_WIFI_CRYPTO_SLOT0 0x600a5800u

#define C6_REG_MODEM_CLOCK_ENABLE 0x600a9814u

#define C6_REG_INTERRUPT_MATRIX_WIFI_ROUTE 0x60010000u
#define C6_REG_PLIC_ENABLE 0x20001000u
#define C6_REG_PLIC_WIFI_PRIORITY 0x20001024u
#define C6_REG_PLIC_THRESHOLD 0x20001090u

#define C6_REG_UART_FIFO 0x60000000u
#define C6_REG_UART_STATUS 0x6000001cu

#endif
