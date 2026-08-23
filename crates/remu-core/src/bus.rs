use crate::SimTime;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Width of a CPU or device bus access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccessWidth {
    /// 8-bit access.
    Byte = 1,
    /// 16-bit access.
    HalfWord = 2,
    /// 32-bit access.
    Word = 4,
    /// 64-bit access.
    DoubleWord = 8,
}

impl AccessWidth {
    /// Width in bytes.
    pub const fn bytes(self) -> u8 {
        self as u8
    }

    /// Mask containing all value bits carried by the access.
    pub const fn value_mask(self) -> u64 {
        match self {
            Self::Byte => 0xff,
            Self::HalfWord => 0xffff,
            Self::Word => 0xffff_ffff,
            Self::DoubleWord => u64::MAX,
        }
    }

    /// Whether an address is naturally aligned for this width.
    pub const fn is_aligned(self, address: u64) -> bool {
        address % self.bytes() as u64 == 0
    }
}

/// Kind of bus operation being attempted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessKind {
    /// Instruction fetch.
    Execute,
    /// Data read.
    Read,
    /// Data write.
    Write,
}

/// Classification of an address-space failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BusFaultKind {
    /// No mapped region contains the requested range.
    Unmapped,
    /// The access crosses a mapped region boundary.
    Boundary,
    /// The region does not permit the requested operation.
    Permission,
    /// The target architecture or device rejects this alignment.
    Misaligned,
    /// A mapped peripheral rejected the access.
    Device,
}

/// Structured address-space failure.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{kind:?} bus fault during {access:?} at {address:#010x} ({width:?}): {message}")]
pub struct BusFault {
    /// Failure classification.
    pub kind: BusFaultKind,
    /// Requested operation.
    pub access: AccessKind,
    /// First requested byte address.
    pub address: u64,
    /// Requested access width.
    pub width: AccessWidth,
    /// Human-readable device or mapping context.
    pub message: String,
}

impl BusFault {
    /// Constructs a new structured bus fault.
    pub fn new(
        kind: BusFaultKind,
        access: AccessKind,
        address: u64,
        width: AccessWidth,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            access,
            address,
            width,
            message: message.into(),
        }
    }
}

/// Address space exposed to interpreted CPUs.
pub trait Bus {
    /// Fetches up to four instruction bytes through an unobserved fast path.
    ///
    /// Implementations return `None` when access observation is enabled or
    /// the address cannot safely supply four bytes; CPUs then use ordinary
    /// architecturally sized reads.
    fn fast_fetch32(&mut self, _address: u64, _at: SimTime) -> Option<Result<u32, BusFault>> {
        None
    }

    /// Reads unobserved data directly from ordinary memory when possible.
    fn fast_read(&mut self, _address: u64, _width: AccessWidth) -> Option<u64> {
        None
    }

    /// Writes unobserved data directly to ordinary memory when possible.
    ///
    /// Returns true when the write completed; false requests the ordinary bus path.
    fn fast_write(&mut self, _address: u64, _width: AccessWidth, _value: u64) -> bool {
        false
    }

    /// Reads data or an instruction from the address space.
    fn read(
        &mut self,
        address: u64,
        width: AccessWidth,
        kind: AccessKind,
        at: SimTime,
    ) -> Result<u64, BusFault>;

    /// Writes data to the address space.
    fn write(
        &mut self,
        address: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), BusFault>;
}
