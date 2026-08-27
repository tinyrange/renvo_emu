use super::PAGE3;
use serde::{Deserialize, Serialize};

/// Named EFM8 PCA and interrupt-control register identifier.
///
/// The EFM8 exposes most PCA registers on SFR pages 0 and 0x10.  The
/// identifier stores the canonical page-0 address (or the explicit extended
/// address for the priority registers), so device code and callers do not
/// have to pass unlabelled integer register IDs around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum Efm8PcaRegister {
    /// PCA control and status flags (PCA0CN0, address 0xd8).
    Pca0Cn = 0xd8,
    /// PCA clock source and overflow interrupt enable (PCA0MD, 0xd9).
    Pca0Md = 0xd9,
    /// PCA channel 0 mode (PCA0CPM0, 0xda).
    Pca0Cpm0 = 0xda,
    /// PCA channel 1 mode (PCA0CPM1, 0xdb).
    Pca0Cpm1 = 0xdb,
    /// PCA channel 2 mode (PCA0CPM2, 0xdc).
    Pca0Cpm2 = 0xdc,
    /// PCA channel output polarity (PCA0POL, 0x96).
    Pca0Pol = 0x96,
    /// PCA PWM cycle length and overflow flags (PCA0PWM, 0xf7).
    Pca0Pwm = 0xf7,
    /// PCA edge/center selection (PCA0CENT, 0xf8).
    Pca0Cent = 0xf8,
    /// PCA counter low byte (PCA0L, 0xf9).
    Pca0L = 0xf9,
    /// PCA counter high byte (PCA0H, 0xfa).
    Pca0H = 0xfa,
    /// PCA channel 0 compare low byte (PCA0CPL0, 0xfb).
    Pca0Cpl0 = 0xfb,
    /// PCA channel 0 compare high byte (PCA0CPH0, 0xfc).
    Pca0Cph0 = 0xfc,
    /// PCA channel 1 compare low byte (PCA0CPL1, 0xe9).
    Pca0Cpl1 = 0xe9,
    /// PCA channel 1 compare high byte (PCA0CPH1, 0xea).
    Pca0Cph1 = 0xea,
    /// PCA channel 2 compare low byte (PCA0CPL2, 0xeb).
    Pca0Cpl2 = 0xeb,
    /// PCA channel 2 compare high byte (PCA0CPH2, 0xec).
    Pca0Cph2 = 0xec,
    /// PCA interrupt enable (EIE1, 0xe6).
    Eie1 = 0xe6,
    /// PCA interrupt priority (EIP1, extended page address 0x10bb).
    Eip1 = 0x10bb,
    /// PCA high-priority interrupt priority (EIP1H, extended page address 0x10ee).
    Eip1h = 0x10ee,
}

impl Efm8PcaRegister {
    /// Stable list of modeled PCA register IDs.
    pub const ALL: [Self; 19] = [
        Self::Pca0Cn,
        Self::Pca0Md,
        Self::Pca0Cpm0,
        Self::Pca0Cpm1,
        Self::Pca0Cpm2,
        Self::Pca0Pol,
        Self::Pca0Pwm,
        Self::Pca0Cent,
        Self::Pca0L,
        Self::Pca0H,
        Self::Pca0Cpl0,
        Self::Pca0Cph0,
        Self::Pca0Cpl1,
        Self::Pca0Cph1,
        Self::Pca0Cpl2,
        Self::Pca0Cph2,
        Self::Eie1,
        Self::Eip1,
        Self::Eip1h,
    ];

    /// Returns the canonical register address used by the device bus.
    pub const fn address(self) -> usize {
        self as usize
    }

    /// Returns the stable debugger/script-facing register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pca0Cn => "pca0cn",
            Self::Pca0Md => "pca0md",
            Self::Pca0Cpm0 => "pca0cpm0",
            Self::Pca0Cpm1 => "pca0cpm1",
            Self::Pca0Cpm2 => "pca0cpm2",
            Self::Pca0Pol => "pca0pol",
            Self::Pca0Pwm => "pca0pwm",
            Self::Pca0Cent => "pca0cent",
            Self::Pca0L => "pca0l",
            Self::Pca0H => "pca0h",
            Self::Pca0Cpl0 => "pca0cpl0",
            Self::Pca0Cph0 => "pca0cph0",
            Self::Pca0Cpl1 => "pca0cpl1",
            Self::Pca0Cph1 => "pca0cph1",
            Self::Pca0Cpl2 => "pca0cpl2",
            Self::Pca0Cph2 => "pca0cph2",
            Self::Eie1 => "eie1",
            Self::Eip1 => "eip1",
            Self::Eip1h => "eip1h",
        }
    }

    /// Resolves a raw SFR address to a named register.
    ///
    /// PCA registers are mirrored on SFR pages 0 and 0x10.  The device's
    /// canonicalisation already resolves those aliases; accepting either
    /// form here makes the public helper useful before canonicalisation too.
    pub fn from_address(address: usize) -> Option<Self> {
        match address {
            0x10bb => Some(Self::Eip1),
            0x10ee => Some(Self::Eip1h),
            _ => match address & 0xff {
                0xd8 => Some(Self::Pca0Cn),
                0xd9 => Some(Self::Pca0Md),
                0xda => Some(Self::Pca0Cpm0),
                0xdb => Some(Self::Pca0Cpm1),
                0xdc => Some(Self::Pca0Cpm2),
                0x96 => Some(Self::Pca0Pol),
                0xf7 => Some(Self::Pca0Pwm),
                0xf8 => Some(Self::Pca0Cent),
                0xf9 => Some(Self::Pca0L),
                0xfa => Some(Self::Pca0H),
                0xfb => Some(Self::Pca0Cpl0),
                0xfc => Some(Self::Pca0Cph0),
                0xe9 => Some(Self::Pca0Cpl1),
                0xea => Some(Self::Pca0Cph1),
                0xeb => Some(Self::Pca0Cpl2),
                0xec => Some(Self::Pca0Cph2),
                0xe6 => Some(Self::Eie1),
                _ => None,
            },
        }
    }
}

const SMB0TC_PAGE3: usize = (PAGE3 << 8) | 0xac;
const SMB0CN0_PAGE3: usize = (PAGE3 << 8) | 0xc0;
const SMB0CF_PAGE3: usize = (PAGE3 << 8) | 0xc1;
const SMB0DAT_PAGE3: usize = (PAGE3 << 8) | 0xc2;
const EIE1_PAGE10: usize = (0x10 << 8) | 0xe6;
const EIP1_ADDRESS: usize = (0x10 << 8) | 0xbb;
const EIP1H_ADDRESS: usize = (0x10 << 8) | 0xee;
const SMB0ADM_PAGE3: usize = (PAGE3 << 8) | 0xd6;
const SMB0ADR_PAGE3: usize = (PAGE3 << 8) | 0xd7;
const SMB0FCN0_ADDRESS: usize = (PAGE3 << 8) | 0xc3;
const SMB0FCN1_ADDRESS: usize = (PAGE3 << 8) | 0xc4;
const SMB0RXLN_ADDRESS: usize = (PAGE3 << 8) | 0xc5;
const SMB0FCT_ADDRESS: usize = (PAGE3 << 8) | 0xef;

/// Named EFM8 SFRs used by the functional SMBus 0 model.
///
/// The SMBus registers are available on SFR page 0 and, where documented,
/// page 3. `offset` returns the canonical bus address used by the device
/// model; `from_data_address` accepts both aliases for dual-page registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Efm8SmbusRegister {
    /// SMBus timing and pin control (SMB0TC).
    Smb0Tc,
    /// SMBus control/status (SMB0CN0).
    Smb0Cn0,
    /// SMBus configuration (SMB0CF).
    Smb0Cf,
    /// SMBus data FIFO access (SMB0DAT).
    Smb0Dat,
    /// SMBus FIFO control 0 (SMB0FCN0).
    Smb0Fcn0,
    /// SMBus FIFO control/status 1 (SMB0FCN1).
    Smb0Fcn1,
    /// SMBus receive length counter (SMB0RXLN).
    Smb0Rxln,
    /// SMBus follower address mask (SMB0ADM).
    Smb0Adm,
    /// SMBus follower address (SMB0ADR).
    Smb0Adr,
    /// Extended interrupt enable 1 (EIE1), including ESMB0.
    Eie1,
    /// Extended interrupt priority 1 low (EIP1), including PSMB0.
    Eip1,
    /// Extended interrupt priority 1 high (EIP1H), including PHSMB0.
    Eip1h,
    /// SMBus FIFO count (SMB0FCT).
    Smb0Fct,
}

impl Efm8SmbusRegister {
    /// Stable debugger/configuration order.
    pub const ALL: [Self; 13] = [
        Self::Smb0Tc,
        Self::Smb0Cn0,
        Self::Smb0Cf,
        Self::Smb0Dat,
        Self::Smb0Fcn0,
        Self::Smb0Fcn1,
        Self::Smb0Rxln,
        Self::Smb0Adm,
        Self::Smb0Adr,
        Self::Eie1,
        Self::Eip1,
        Self::Eip1h,
        Self::Smb0Fct,
    ];

    /// Canonical byte address in the paged SFR bus.
    pub const fn offset(self) -> usize {
        match self {
            Self::Smb0Tc => 0xac,
            Self::Smb0Cn0 => 0xc0,
            Self::Smb0Cf => 0xc1,
            Self::Smb0Dat => 0xc2,
            Self::Smb0Fcn0 => (PAGE3 << 8) | 0xc3,
            Self::Smb0Fcn1 => (PAGE3 << 8) | 0xc4,
            Self::Smb0Rxln => (PAGE3 << 8) | 0xc5,
            Self::Smb0Adm => 0xd6,
            Self::Smb0Adr => 0xd7,
            Self::Eie1 => 0xe6,
            Self::Eip1 => (0x10 << 8) | 0xbb,
            Self::Eip1h => (0x10 << 8) | 0xee,
            Self::Smb0Fct => (PAGE3 << 8) | 0xef,
        }
    }

    /// Stable numeric index for tables and serialized register metadata.
    pub const fn index(self) -> usize {
        match self {
            Self::Smb0Tc => 0,
            Self::Smb0Cn0 => 1,
            Self::Smb0Cf => 2,
            Self::Smb0Dat => 3,
            Self::Smb0Fcn0 => 4,
            Self::Smb0Fcn1 => 5,
            Self::Smb0Rxln => 6,
            Self::Smb0Adm => 7,
            Self::Smb0Adr => 8,
            Self::Eie1 => 9,
            Self::Eip1 => 10,
            Self::Eip1h => 11,
            Self::Smb0Fct => 12,
        }
    }

    /// Stable lowercase register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Smb0Tc => "smb0tc",
            Self::Smb0Cn0 => "smb0cn0",
            Self::Smb0Cf => "smb0cf",
            Self::Smb0Dat => "smb0dat",
            Self::Smb0Fcn0 => "smb0fcn0",
            Self::Smb0Fcn1 => "smb0fcn1",
            Self::Smb0Rxln => "smb0rxln",
            Self::Smb0Adm => "smb0adm",
            Self::Smb0Adr => "smb0adr",
            Self::Eie1 => "eie1",
            Self::Eip1 => "eip1",
            Self::Eip1h => "eip1h",
            Self::Smb0Fct => "smb0fct",
        }
    }

    /// Resolves a raw paged-SFR address to its named SMBus register.
    pub fn from_data_address(address: usize) -> Option<Self> {
        match address {
            0xac | SMB0TC_PAGE3 => Some(Self::Smb0Tc),
            0xc0 | SMB0CN0_PAGE3 => Some(Self::Smb0Cn0),
            0xc1 | SMB0CF_PAGE3 => Some(Self::Smb0Cf),
            0xc2 | SMB0DAT_PAGE3 => Some(Self::Smb0Dat),
            SMB0FCN0_ADDRESS => Some(Self::Smb0Fcn0),
            SMB0FCN1_ADDRESS => Some(Self::Smb0Fcn1),
            SMB0RXLN_ADDRESS => Some(Self::Smb0Rxln),
            0xd6 | SMB0ADM_PAGE3 => Some(Self::Smb0Adm),
            0xd7 | SMB0ADR_PAGE3 => Some(Self::Smb0Adr),
            0xe6 | EIE1_PAGE10 => Some(Self::Eie1),
            EIP1_ADDRESS => Some(Self::Eip1),
            EIP1H_ADDRESS => Some(Self::Eip1h),
            SMB0FCT_ADDRESS => Some(Self::Smb0Fct),
            _ => None,
        }
    }
}
