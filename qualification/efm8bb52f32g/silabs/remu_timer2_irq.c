// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

SI_SBIT(remu_led, SFR_P1, 4);

SI_INTERRUPT(remu_timer2_interrupt, TIMER2_IRQn)
{
    TMR2CN0_TF2H = 0;
    remu_led = !remu_led;
}
