// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level Timer0 fixture based only on the
 * PIC16F15376 data sheet. It exercises oscillator setup, digital output,
 * Timer0 interrupt routing, and an observable RE0 transition.
 */
static void configure_fixture(void)
{
    OSCCON1 = 0x60;
    OSCFRQ = 0x00;
    TRISEbits.TRISE0 = 0;
    LATEbits.LATE0 = 0;

    T0CON1 = 0x94;
    TMR0H = 0xC1;
    TMR0L = 0x00;
    PIR0bits.TMR0IF = 0;
    PIE0bits.TMR0IE = 1;
    T0CON0 = 0x80;

    INTCONbits.PEIE = 1;
    INTCONbits.GIE = 1;
}

void __interrupt() remu_interrupt(void)
{
    if (PIE0bits.TMR0IE && PIR0bits.TMR0IF) {
        PIR0bits.TMR0IF = 0;
        LATEbits.LATE0 ^= 1;
    }
}

void main(void)
{
    configure_fixture();
    for (;;) {
        /* Timer0 supplies the observable event. */
    }
}
