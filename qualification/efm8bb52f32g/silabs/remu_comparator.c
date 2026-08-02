// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include "SI_EFM8BB52_Register_Enums.h"

void main(void)
{
    CMP0MX = 0x12;
    CMP0CN1 = 0x20; /* enable the internal reference-DAC path */
    CMP0MD = 0x30;  /* rising and falling edge interrupts */
    CMP0CN0 = 0x80;
    CMP1MX = 0x21;
    CMP1CN1 = 0x20;
    CMP1MD = 0x30;
    CMP1CN0 = 0x80;
    EIE1 = 0x60;
    for (;;) {
    }
}

