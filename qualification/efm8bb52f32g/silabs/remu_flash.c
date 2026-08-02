// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * The EFM8 flash controller accepts MOVX stores only after the documented
 * FLKEY sequence. A PSEE store erases its complete 2 KiB page; a PSWE-only
 * store programs bits from one to zero. This fixture exercises both paths
 * through the firmware-visible XDATA address space.
 */
void main(void)
{
    volatile __xdata uint8_t *flash = (volatile __xdata uint8_t *)0x1000u;

    enter_DefaultMode_from_RESET();
    P1 = 0xffu;
    P1MDOUT = 0x10u;
    P1 = 0u;

    FLKEY = 0xa5u;
    FLKEY = 0xf1u;
    PSCTL = 0x03u;
    flash[0] = 0u;

    FLKEY = 0xa5u;
    FLKEY = 0xf1u;
    PSCTL = 0x01u;
    flash[0] = 0x5au;
    PSCTL = 0u;

    P1 = (flash[0] == 0x5au) ? 0x10u : 0u;
    for (;;) {
    }
}
