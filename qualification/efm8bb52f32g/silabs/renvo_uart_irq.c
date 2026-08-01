// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo contributors

#include <SI_EFM8BB52_Register_Enums.h>

static uint8_t renvo_received;

/* Compile-only ABI fixture for the EFM8 UART0 interrupt vector. */
SI_INTERRUPT(renvo_uart0_interrupt, UART0_IRQn)
{
    if (SCON0_RI) {
        SCON0_RI = 0;
        renvo_received = SBUF0;
        if (renvo_received >= 'a' && renvo_received <= 'z') {
            renvo_received = (uint8_t)(renvo_received - ('a' - 'A'));
        }
        SBUF0 = renvo_received;
    }
    if (SCON0_TI) {
        SCON0_TI = 0;
    }
}
