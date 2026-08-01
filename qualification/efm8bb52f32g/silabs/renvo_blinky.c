// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * Original Renvo register-level fixture.  The register choices and reset
 * values come from the public EFM8BB52 reference manual; no SDK source is
 * included or required.
 */
void SiLabs_Startup(void)
{
    /* SDCC calls this hook before main. Configuration is performed below. */
}

void main(void)
{
    enter_DefaultMode_from_RESET();
    IE_EA = 1;
    for (;;) {
        /* Timer2 drives the observable P1.4 transition. */
    }
}
