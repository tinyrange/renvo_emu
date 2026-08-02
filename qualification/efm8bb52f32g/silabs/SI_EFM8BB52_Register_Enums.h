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
#define TIMER3_IRQn 14
#define TIMER4_IRQn 17
#define TIMER5_IRQn 18
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
/* Timer3 is mirrored on SFR pages 0x00 and 0x10. */
__sfr __at (0x91) TMR3CN0;
__sfr __at (0x92) TMR3RLL;
__sfr __at (0x93) TMR3RLH;
__sfr __at (0x94) TMR3L;
__sfr __at (0x95) TMR3H;
/* Timer4 and Timer5 are selected with SFRPAGE=0x10. */
__sfr __at (0x98) TMR4CN0;
__sfr __at (0xa2) TMR4RLL;
__sfr __at (0xa3) TMR4RLH;
__sfr __at (0xa4) TMR4L;
__sfr __at (0xa5) TMR4H;
__sfr __at (0xc0) TMR5CN0;
__sfr __at (0xd2) TMR5RLL;
__sfr __at (0xd3) TMR5RLH;
__sfr __at (0xd4) TMR5L;
__sfr __at (0xd5) TMR5H;
__sfr __at (0xe6) EIE1;
__sfr __at (0xf3) EIE2;
__sfr __at (0xbb) EIP1;
__sfr __at (0xee) EIP1H;
__sfr __at (0xed) EIP2;
__sfr __at (0xf6) EIP2H;
__sfr __at (0xa4) P0MDOUT;
__sfr __at (0xa5) P1MDOUT;
__sfr __at (0xa7) SFRPAGE;
__sfr __at (0xa8) IE;
__sfr __at (0xc8) TMR2CN0;
__sfr __at (0xca) TMR2RLL;
__sfr __at (0xcb) TMR2RLH;
__sfr __at (0xce) TMR2L;
__sfr __at (0xcf) TMR2H;
__sfr __at (0xe1) XBR0;
__sfr __at (0xe3) XBR2;
__sfr __at (0xf1) P0MDIN;
__sfr __at (0xf2) P1MDIN;

__sbit __at (0xaf) IE_EA;
__sbit __at (0x98) SCON0_RI;
__sbit __at (0x99) SCON0_TI;
__sbit __at (0xcf) TMR2CN0_TF2H;

#endif
