// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level Timer2 fixture based only on the
 * PIC16F15376 data sheet. It exercises the programmable Timer2 period,
 * prescaler/postscaler, and the shared peripheral interrupt vector.
 */
static void configure_fixture(void)
{
    OSCCON1 = 0x60;
    OSCFRQ = 0x00;
    TRISEbits.TRISE0 = 0;
    LATEbits.LATE0 = 0;

    T2TMR = 0;
    T2PR = 0x03;
    T2CLKCON = 0x01; /* FOSC/4 */
    T2CON = 0x80;    /* Timer2 on, 1:1 prescale and postscale */
    PIR4bits.TMR2IF = 0;
    PIE4bits.TMR2IE = 1;

    INTCONbits.PEIE = 1;
    INTCONbits.GIE = 1;
}

void __interrupt() remu_interrupt(void)
{
    if (PIE4bits.TMR2IE && PIR4bits.TMR2IF) {
        PIR4bits.TMR2IF = 0;
        LATEbits.LATE0 ^= 1;
    }
}

void main(void)
{
    configure_fixture();
    for (;;) {
        /* Timer2 supplies the observable event. */
    }
}
