// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level NCO1 fixture based only on the
 * PIC16F15376 data sheet. A half-scale fixed-duty increment makes the first
 * accumulator overflow observable at the functional two-tick loop boundary.
 */
void main(void)
{
    NCO1INCU = 0x08;
    NCO1INCH = 0x00;
    NCO1INCL = 0x00;
    NCO1CON = 0x80; /* fixed-duty mode, enabled */

    for (;;) {
        /* The qualification harness stops on the first NCO1 output edge. */
    }
}
