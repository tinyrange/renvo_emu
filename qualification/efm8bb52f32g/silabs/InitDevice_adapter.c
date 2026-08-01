// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * Project-authored setup for the original Renvo register fixtures. The short
 * Timer2 reload keeps the functional-time qualification bounded.
 */
void enter_DefaultMode_from_RESET(void)
{
    WDTCN = 0xdeu;
    WDTCN = 0xadu;
    SFRPAGE = 0u;
    P1 = 0xffu;
    P1MDIN = 0xffu;
    P1MDOUT = 0x10u;
    XBR2 = 0x40u;
    TMR2RLL = 0xc0u;
    TMR2RLH = 0xffu;
    TMR2L = 0xc0u;
    TMR2H = 0xffu;
    TMR2CN0 = 0x04u;
    IE = 0x20u;
}
