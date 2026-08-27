// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

static uint8_t remu_timer_flags;

/* Compile-only ABI fixture for the extended EFM8 timer vector slots. */
SI_INTERRUPT(remu_timer3_interrupt, TIMER3_IRQn)
{
    remu_timer_flags |= TMR3CN0;
    TMR3CN0 &= (uint8_t)~0xc0;
}

SI_INTERRUPT(remu_timer4_interrupt, TIMER4_IRQn)
{
    remu_timer_flags |= TMR4CN0;
    TMR4CN0 &= (uint8_t)~0xc0;
}

SI_INTERRUPT(remu_timer5_interrupt, TIMER5_IRQn)
{
    remu_timer_flags |= TMR5CN0;
    TMR5CN0 &= (uint8_t)~0xc0;
}
