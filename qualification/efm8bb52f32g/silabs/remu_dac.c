// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include "SI_EFM8BB52_Register_Enums.h"

void main(void)
{
    SFRPAGE = 0x30;
    DAC0CF0 = 0x80; /* enable, SYSCLK software update */
    DAC0L = 0x5a;
    DAC0H = 0x02;
    for (;;) {
    }
}

