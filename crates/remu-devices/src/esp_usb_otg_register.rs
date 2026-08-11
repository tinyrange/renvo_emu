//! Named register identifiers for the ESP32-S3 DWC2 USB OTG block.

/// Register identifiers implemented by the ESP32-S3 DWC2 USB OTG slice.
///
/// Endpoint identifiers carry the endpoint number and map through the DWC2
/// `0x20` stride. Unlisted registers retain backing register-file behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum EspUsbOtgRegister {
    GotgInt,
    GahbCfg,
    Gusbcfg,
    GrstCtl,
    GintSts,
    GintMsk,
    GrxStsR,
    GrxStsP,
    GrxFsiz,
    GnptxFsiz,
    GsnpsId,
    GhwCfg2,
    GdfifoCfg,
    HptxFsiz,
    DiepTxFifo(u8),
    Dcfg,
    Dctl,
    Dsts,
    DiepMsk,
    DoepMsk,
    Daint,
    DaintMsk,
    DiepEmpMsk,
    DiepCtl(u8),
    DiepInt(u8),
    DiepTsiz(u8),
    DtxfSts(u8),
    DoepCtl(u8),
    DoepInt(u8),
    DoepTsiz(u8),
    Fifo(u8),
}

impl EspUsbOtgRegister {
    /// Converts a DWC2 register offset into a named identifier.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x04 => return Some(Self::GotgInt),
            0x08 => return Some(Self::GahbCfg),
            0x0c => return Some(Self::Gusbcfg),
            0x10 => return Some(Self::GrstCtl),
            0x14 => return Some(Self::GintSts),
            0x18 => return Some(Self::GintMsk),
            0x1c => return Some(Self::GrxStsR),
            0x20 => return Some(Self::GrxStsP),
            0x24 => return Some(Self::GrxFsiz),
            0x28 => return Some(Self::GnptxFsiz),
            0x40 => return Some(Self::GsnpsId),
            0x48 => return Some(Self::GhwCfg2),
            0x5c => return Some(Self::GdfifoCfg),
            0x100 => return Some(Self::HptxFsiz),
            0x104 => return Some(Self::DiepTxFifo(1)),
            0x108 => return Some(Self::DiepTxFifo(2)),
            0x10c => return Some(Self::DiepTxFifo(3)),
            0x110 => return Some(Self::DiepTxFifo(4)),
            0x800 => return Some(Self::Dcfg),
            0x804 => return Some(Self::Dctl),
            0x808 => return Some(Self::Dsts),
            0x810 => return Some(Self::DiepMsk),
            0x814 => return Some(Self::DoepMsk),
            0x818 => return Some(Self::Daint),
            0x81c => return Some(Self::DaintMsk),
            0x834 => return Some(Self::DiepEmpMsk),
            _ => {}
        }
        if offset >= 0x900 && offset < 0xb00 {
            let endpoint = ((offset - 0x900) / 0x20) as u8;
            return match (offset - 0x900) % 0x20 {
                0 => Some(Self::DiepCtl(endpoint)),
                8 => Some(Self::DiepInt(endpoint)),
                16 => Some(Self::DiepTsiz(endpoint)),
                24 => Some(Self::DtxfSts(endpoint)),
                _ => None,
            };
        }
        if offset >= 0xb00 && offset < 0xd00 {
            let endpoint = ((offset - 0xb00) / 0x20) as u8;
            return match (offset - 0xb00) % 0x20 {
                0 => Some(Self::DoepCtl(endpoint)),
                8 => Some(Self::DoepInt(endpoint)),
                16 => Some(Self::DoepTsiz(endpoint)),
                _ => None,
            };
        }
        if offset >= 0x1000 && offset < 0x1_0000 && (offset - 0x1000) % 0x1000 == 0 {
            return Some(Self::Fifo(((offset - 0x1000) / 0x1000) as u8));
        }
        None
    }

    /// Returns the DWC2 byte offset for this register identifier.
    pub const fn offset(self) -> u64 {
        match self {
            Self::GotgInt => 0x04,
            Self::GahbCfg => 0x08,
            Self::Gusbcfg => 0x0c,
            Self::GrstCtl => 0x10,
            Self::GintSts => 0x14,
            Self::GintMsk => 0x18,
            Self::GrxStsR => 0x1c,
            Self::GrxStsP => 0x20,
            Self::GrxFsiz => 0x24,
            Self::GnptxFsiz => 0x28,
            Self::GsnpsId => 0x40,
            Self::GhwCfg2 => 0x48,
            Self::GdfifoCfg => 0x5c,
            Self::HptxFsiz => 0x100,
            Self::DiepTxFifo(endpoint) => 0x100 + endpoint as u64 * 4,
            Self::Dcfg => 0x800,
            Self::Dctl => 0x804,
            Self::Dsts => 0x808,
            Self::DiepMsk => 0x810,
            Self::DoepMsk => 0x814,
            Self::Daint => 0x818,
            Self::DaintMsk => 0x81c,
            Self::DiepEmpMsk => 0x834,
            Self::DiepCtl(endpoint) => 0x900 + endpoint as u64 * 0x20,
            Self::DiepInt(endpoint) => 0x908 + endpoint as u64 * 0x20,
            Self::DiepTsiz(endpoint) => 0x910 + endpoint as u64 * 0x20,
            Self::DtxfSts(endpoint) => 0x918 + endpoint as u64 * 0x20,
            Self::DoepCtl(endpoint) => 0xb00 + endpoint as u64 * 0x20,
            Self::DoepInt(endpoint) => 0xb08 + endpoint as u64 * 0x20,
            Self::DoepTsiz(endpoint) => 0xb10 + endpoint as u64 * 0x20,
            Self::Fifo(endpoint) => 0x1000 + endpoint as u64 * 0x1000,
        }
    }

    pub(crate) const fn index(self) -> usize {
        (self.offset() / 4) as usize
    }
}
