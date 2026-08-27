// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 Renvo Emulator contributors

#include <SI_EFM8BB52_Register_Enums.h>

/*
 * Original Renvo Emulator register-level SMBus fixture. It drives the
 * documented leader-start/data path; the host observes the byte transaction
 * through the SMBus VCD signals. No Silicon Labs SDK source is required.
 */
void SiLabs_Startup(void)
{
}

void main(void)
{
    WDTCN = 0xdeu;
    WDTCN = 0xadu;
    SMB0CF = 0x80u; /* ENSMB */
    EIE1 = 0x01u;   /* ESMB0 */
    SMB0CN0 = 0x20u; /* leader START */
    SMB0CN0 = 0x00u; /* firmware services SI */
    SMB0DAT = 0xa0u; /* first address/data byte */
    for (;;) {
        /* The host stops on the SMB0 transmit strobe. */
    }
}
