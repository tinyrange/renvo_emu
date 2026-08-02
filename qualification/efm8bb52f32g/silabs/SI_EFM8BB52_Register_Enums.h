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
#define PCA0_IRQn 6
#define UART1_IRQn 15
#define TIMER3_IRQn 14
#define TIMER4_IRQn 17
#define TIMER5_IRQn 18
#define ADC_WINDOW_IRQn 9
#define ADC0_IRQn 10
#define CL0_IRQn 19
#define SFR_P0 0x80
#define SFR_P1 0x90
#define SFR_P2 0xa0
#define SFR_P3 0xb0

__sfr __at (0x80) P0;
__sfr __at (0x88) TCON;
__sfr __at (0x89) TMOD;
__sfr __at (0x8a) TL0;
__sfr __at (0x8c) TH0;
__sfr __at (0x8f) PSCTL;
__sfr __at (0x90) P1;
__sfr __at (0x97) WDTCN;
__sfr __at (0x98) SCON0;
__sfr __at (0x99) SBUF0;
__sfr __at (0xc0) SMB0CN0;
__sfr __at (0xc1) SMB0CF;
__sfr __at (0xc2) SMB0DAT;
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
__sfr __at (0xb2) ADC0CN1;
__sfr __at (0xb3) ADC0CN2;
__sfr __at (0xb9) ADC0CF1;
__sfr __at (0xbb) ADC0MX;
__sfr __at (0xbd) ADC0L;
__sfr __at (0xbe) ADC0H;
__sfr __at (0xc3) ADC0GTL;
__sfr __at (0xc4) ADC0GTH;
__sfr __at (0xc5) ADC0LTL;
__sfr __at (0xc6) ADC0LTH;
__sfr __at (0xdf) ADC0CF2;
__sfr __at (0xe8) ADC0CN0;
/* DAC0 is selected with SFRPAGE=0x30. */
__sfr __at (0x84) DAC0L;
__sfr __at (0x85) DAC0H;
__sfr __at (0x8a) DAC0ALT;
__sfr __at (0x91) DAC0CF0;
__sfr __at (0x92) DAC0CF1;
/* Comparators are selected with SFRPAGE=0x30. */
__sfr __at (0x99) CMP0CN1;
__sfr __at (0x9b) CMP0CN0;
__sfr __at (0x9d) CMP0MD;
__sfr __at (0x9f) CMP0MX;
__sfr __at (0xaa) CMP1MX;
__sfr __at (0xab) CMP1MD;
__sfr __at (0xac) CMP1CN1;
__sfr __at (0xbf) CMP1CN0;
/* CLU0-3 are selected with SFRPAGE=0x20. */
__sfr __at (0xc6) CLEN0;
__sfr __at (0xc7) CLIE0;
__sfr __at (0xe8) CLIF0;
__sfr __at (0xd1) CLOUT0;
__sfr __at (0x84) CLU0MX;
__sfr __at (0xaf) CLU0FN;
__sfr __at (0xb1) CLU0CF;
__sfr __at (0xfd) P0MAT;
__sfr __at (0xfe) P0MASK;
#define EIE1_EMAT 0x02
__sfr __at (0xf3) EIE2;
__sfr __at (0xbb) EIP1;
__sfr __at (0xee) EIP1H;
__sfr __at (0xed) EIP2;
__sfr __at (0xf6) EIP2H;
/* UART1 is on SFR page 0x20; firmware selects that page through SFRPAGE. */
__sfr __at (0x92) SBUF1;
__sfr __at (0x93) SMOD1;
__sfr __at (0x94) SBCON1;
__sfr __at (0x95) SBRLL1;
__sfr __at (0x96) SBRLH1;
__sfr __at (0xc8) SCON1;
__sfr __at (0x9d) UART1FCN0;
__sfr __at (0xd8) UART1FCN1;
__sfr __at (0xfa) UART1FCT;
__sfr __at (0xa4) P0MDOUT;
__sfr __at (0xa5) P1MDOUT;
__sfr __at (0xa7) SFRPAGE;
__sfr __at (0xa8) IE;
__sfr __at (0xb7) FLKEY;
__sfr __at (0xc8) TMR2CN0;
__sfr __at (0xca) TMR2RLL;
__sfr __at (0xcb) TMR2RLH;
__sfr __at (0xce) TMR2L;
__sfr __at (0xcf) TMR2H;
__sfr __at (0xd8) PCA0CN;
__sfr __at (0xd9) PCA0MD;
__sfr __at (0xda) PCA0CPM0;
__sfr __at (0xdb) PCA0CPM1;
__sfr __at (0xdc) PCA0CPM2;
__sfr __at (0xe6) EIE1;
__sfr __at (0xe1) XBR0;
__sfr __at (0xe3) XBR2;
__sfr __at (0xf7) PCA0PWM;
__sfr __at (0xf8) PCA0CENT;
__sfr __at (0xf9) PCA0L;
__sfr __at (0xfa) PCA0H;
__sfr __at (0xfb) PCA0CPL0;
__sfr __at (0xfc) PCA0CPH0;
__sfr __at (0xe9) PCA0CPL1;
__sfr __at (0xea) PCA0CPH1;
__sfr __at (0xeb) PCA0CPL2;
__sfr __at (0xec) PCA0CPH2;
__sfr __at (0xf1) P0MDIN;
__sfr __at (0xf2) P1MDIN;

__sbit __at (0xaf) IE_EA;
__sbit __at (0x98) SCON0_RI;
__sbit __at (0x99) SCON0_TI;
__sbit __at (0xcf) TMR2CN0_TF2H;
__sbit __at (0xd8) PCA0CN_CF;
__sbit __at (0xd9) PCA0MD_ECF;
__sbit __at (0xe6) EIE1_EPCA0;
__sbit __at (0xc8) SCON1_RI;
__sbit __at (0xc9) SCON1_TI;
__sbit __at (0xcc) SCON1_REN;

#endif
