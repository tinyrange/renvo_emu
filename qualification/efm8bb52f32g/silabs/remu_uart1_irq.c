// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

static uint8_t remu_uart1_received;

/* Compile-only ABI fixture for the paged EFM8 UART1 interrupt vector. */
SI_INTERRUPT(remu_uart1_interrupt, UART1_IRQn)
{
    if (SCON1_RI) {
        SCON1_RI = 0;
        remu_uart1_received = SBUF1;
        SBUF1 = remu_uart1_received;
    }
    if (SCON1_TI) {
        SCON1_TI = 0;
    }
}
