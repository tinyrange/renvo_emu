// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * UART0 has fixed P0.4/P0.5 pins and priority over the assignable SPI0
 * resources. P0.0 is skipped so SPI0 begins at P0.1 and its fourth signal
 * lands after the UART pins at P0.6. The VCD crossbar route signals expose the
 * deterministic allocation made by these writes.
 */
void main(void)
{
    enter_DefaultMode_from_RESET();
    P0SKIP = 0x01u;
    XBR0 = 0x03u;
    XBR1 = 0x00u;
    XBR2 = 0x40u;
    for (;;) {
    }
}
