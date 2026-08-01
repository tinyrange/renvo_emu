// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

static uint8_t remu_received;

/* Compile-only ABI fixture for the EFM8 UART0 interrupt vector. */
SI_INTERRUPT(remu_uart0_interrupt, UART0_IRQn)
{
    if (SCON0_RI) {
        SCON0_RI = 0;
        remu_received = SBUF0;
        if (remu_received >= 'a' && remu_received <= 'z') {
            remu_received = (uint8_t)(remu_received - ('a' - 'A'));
        }
        SBUF0 = remu_received;
    }
    if (SCON0_TI) {
        SCON0_TI = 0;
    }
}
