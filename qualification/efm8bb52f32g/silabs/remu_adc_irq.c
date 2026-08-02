// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include "SI_EFM8BB52_Register_Enums.h"

static volatile uint16_t remu_adc_sample;

SI_INTERRUPT(remu_adc_window_interrupt, ADC_WINDOW_IRQn)
{
    /* ADWINT is a latched status bit on the modeled ADC0CN0 surface. */
    ADC0CN0 &= (uint8_t)~0x08;
}

SI_INTERRUPT(remu_adc_interrupt, ADC0_IRQn)
{
    if ((ADC0CN0 & 0x20) != 0) {
        remu_adc_sample = ((uint16_t)ADC0H << 8) | ADC0L;
        ADC0CN0 &= (uint8_t)~0x20;
    }
}

