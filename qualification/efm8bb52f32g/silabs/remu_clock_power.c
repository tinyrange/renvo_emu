// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * The source/divider values are from CLKSEL in the public EFM8BB52 reference
 * manual. The fixture then requests SNOOZE, which is a deterministic stop in
 * the functional model rather than a claim about oscillator settle timing.
 */
void main(void)
{
    enter_DefaultMode_from_RESET();
    CLKSEL = 0x12u; /* LFOSC0 at 80 kHz divided by two. */
    PCON1 = 0x80u;  /* SNOOZE. */
    for (;;) {
    }
}
