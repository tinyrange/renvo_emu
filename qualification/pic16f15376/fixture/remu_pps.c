// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator PPS output fixture based on the PIC16F15376 data
 * sheet. It routes the functional Timer0 output source to RA0; the host checks
 * that the selected peripheral source, rather than LATA, drives the pin.
 */
void main(void)
{
    ANSELA = 0x00;
    TRISA = 0xfe;
    RA0PPS = 0x19; /* TMR0 output source */
    TMR0H = 0x01;
    T0CON0 = 0x80;
    for (;;) {
        /* The host stops on the routed RA0 transition. */
    }
}
