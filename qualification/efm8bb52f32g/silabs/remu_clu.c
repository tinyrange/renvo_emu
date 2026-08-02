// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/* Register-level CLU fixture using the documented A AND B truth table. */
void main(void)
{
    enter_DefaultMode_from_RESET();
    SFRPAGE = 0x20u;
    CLU0MX = 0x88u; /* P0.0 -> A, P0.1 -> B. */
    CLU0FN = 0xc0u; /* A AND B. */
    CLU0CF = 0x80u; /* Select the LUT output. */
    CLEN0 = 0x01u;
    if (CLOUT0 & 0x01u) {
        SFRPAGE = 0u;
        P1 = 0x10u;
    }
    for (;;) {
        /* The host drives P0.0 and P0.1; the qualified run stops on P1.4. */
    }
}
