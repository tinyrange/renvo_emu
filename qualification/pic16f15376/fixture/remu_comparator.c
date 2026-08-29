// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level comparator fixture based only on
 * the PIC16F15376 data sheet. The host drives RA0 (C1IN0-) and RA2 (C1IN0+).
 */
void main(void)
{
    CM1NCH = 0x00; /* C1IN0- on RA0 */
    CM1PCH = 0x00; /* C1IN0+ on RA2 */
    CM1CON0 = 0x80;
    for (;;) {
        /* C1 output is observable through the comparator signal. */
    }
}
