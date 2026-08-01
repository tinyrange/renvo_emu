#include <SI_EFM8BB52_Register_Enums.h>
#include <InitDevice.h>

/*
 * Allowed compiler/board adapter: the upstream Hardware Configurator file is
 * replaced with a small equivalent setup so the official main and ISR remain
 * byte-for-byte source compatible with SDCC and finish quickly in functional
 * time. The selected Timer2/GPIO peripheral logic is not changed.
 */
void enter_DefaultMode_from_RESET(void)
{
    WDTCN = 0xdeu;
    WDTCN = 0xadu;
    SFRPAGE = 0u;
    P1 = 0xffu;
    P1MDIN = 0xffu;
    P1MDOUT = 0x10u;
    XBR2 = 0x40u;
    TMR2RLL = 0xc0u;
    TMR2RLH = 0xffu;
    TMR2L = 0xc0u;
    TMR2H = 0xffu;
    TMR2CN0 = 0x04u;
    IE = 0x20u;
}
