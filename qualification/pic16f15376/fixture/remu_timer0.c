// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <xc.h>

#pragma config WDTE = OFF
#pragma config LVP = ON

/*
 * Original Renvo Emulator register-level fixture based only on the PIC16F15376
 * data sheet. It exercises oscillator setup, digital output, the functional
 * MSSP1 I2C host path, Timer0 interrupt routing, and an observable RE0
 * transition.
 */
static void configure_fixture(void)
{
    OSCCON1 = 0x60;
    OSCFRQ = 0x00;
    TRISEbits.TRISE0 = 0;
    LATEbits.LATE0 = 0;

    /* 7-bit MSSP1 I2C master: START, address, one data byte, STOP. */
    SSP1CON1 = 0x28;
    SSP1CON2 = 0x01;
    SSP1BUF = 0xa0;
    PIR3bits.SSP1IF = 0;
    SSP1BUF = 0x10;
    PIR3bits.SSP1IF = 0;
    SSP1CON2 = 0x04;
    PIR3bits.SSP1IF = 0;

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
