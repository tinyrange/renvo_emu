// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level DAC fixture based only on the
 * PIC16F15376 data sheet. It exercises the 5-bit range and DAC enable/output
 * controls; the emulator exposes the normalized code as a waveform signal.
 */
void main(void)
{
    DAC1CON1 = 0x15;
    DAC1CON0 = 0xA0; /* enable DAC1 and route the code to output 1 */
    for (;;) {
        /* DAC1 supplies the observable analog-code state. */
    }
}
