// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#ifndef REMU_SI_EFM8BB52_REGISTER_ENUMS_H
#define REMU_SI_EFM8BB52_REGISTER_ENUMS_H

#include <stdint.h>

/* SDCC declarations for the original Renvo Emulator EFM8BB52 register fixtures. */
#define SI_SBIT(name, address, bit) __sbit __at ((address) + (bit)) name
#define SI_INTERRUPT(name, vector) void name(void) __interrupt (vector)

#define TIMER0_IRQn 1
#define UART0_IRQn 4
#define TIMER2_IRQn 5
#define SFR_P0 0x80
#define SFR_P1 0x90
#define SFR_P2 0xa0
#define SFR_P3 0xb0

__sfr __at (0x80) P0;
__sfr __at (0x88) TCON;
__sfr __at (0x89) TMOD;
__sfr __at (0x8a) TL0;
__sfr __at (0x8c) TH0;
__sfr __at (0x90) P1;
__sfr __at (0x97) WDTCN;
__sfr __at (0x98) SCON0;
__sfr __at (0x99) SBUF0;
__sfr __at (0xa4) P0MDOUT;
__sfr __at (0xa5) P1MDOUT;
__sfr __at (0xa7) SFRPAGE;
__sfr __at (0xa8) IE;
__sfr __at (0xc8) TMR2CN0;
__sfr __at (0xca) TMR2RLL;
__sfr __at (0xcb) TMR2RLH;
__sfr __at (0xce) TMR2L;
__sfr __at (0xcf) TMR2H;
__sfr __at (0x99) CMP0CN1;
__sfr __at (0x9b) CMP0CN0;
__sfr __at (0x9d) CMP0MD;
__sfr __at (0x9f) CMP0MX;
__sfr __at (0xaa) CMP1MX;
__sfr __at (0xab) CMP1MD;
__sfr __at (0xac) CMP1CN1;
__sfr __at (0xbf) CMP1CN0;
__sfr __at (0xe6) EIE1;
__sfr __at (0xe1) XBR0;
__sfr __at (0xe3) XBR2;
__sfr __at (0xf1) P0MDIN;
__sfr __at (0xf2) P1MDIN;

__sbit __at (0xaf) IE_EA;
__sbit __at (0x98) SCON0_RI;
__sbit __at (0x99) SCON0_TI;
__sbit __at (0xcf) TMR2CN0_TF2H;

#endif
