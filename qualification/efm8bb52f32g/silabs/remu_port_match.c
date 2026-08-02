// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/* Register-level port-match fixture for an active-low P0.0 input. */
void main(void)
{
    enter_DefaultMode_from_RESET();
    P0MAT = 0x01u;
    P0MASK = 0x01u;
    EIE1 = EIE1_EMAT;
    IE = 0x80u;
    for (;;) {
        /* The host drives P0.0 low; the mismatch signal is externally observed. */
    }
}
