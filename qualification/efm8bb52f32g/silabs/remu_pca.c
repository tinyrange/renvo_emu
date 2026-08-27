// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

volatile uint8_t remu_pca_matches;

SI_INTERRUPT(remu_pca_interrupt, PCA0_IRQn)
{
    PCA0CN &= (uint8_t)~0x01u;
    remu_pca_matches++;
}

void remu_pca_configure(void)
{
    /* SYSCLK timebase, edge-aligned 8-bit PWM, 25% duty on CEX0. */
    PCA0MD = 0x08u;
    PCA0PWM = 0x00u;
    PCA0CPM0 = 0x02u;
    PCA0CPL0 = 0x40u;
    PCA0CPH0 = 0x00u;
    EIE1 = 0x10u;
    IE |= 0x80u;
    PCA0CN = 0x40u;
}
