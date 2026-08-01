//! Firmware containers, official artifact manifests, and persistent flash images.

mod esp;
mod flash;
mod ihex;
mod manifest;
mod uf2;

pub use esp::{
    EspExecutableImage, EspFlashImage, EspImageError, EspImageHeader, EspImageSegment,
    EspPartition, EspPartitionTable,
};
pub use flash::{FlashError, FlashSnapshot, PersistentFlash};
pub use ihex::{
    IntelHexError, IntelHexImage, IntelHexRecord, IntelHexRecordType, IntelHexSegment,
    ProgramWordEndianness, ProgramWordImage, ProgramWordSegment,
};
pub use manifest::{
    FirmwareArtifactFormat, FirmwareManifestError, OfficialFirmwareArtifact, OfficialFirmwareSuite,
    VerifiedFirmwareArtifact,
};
pub use uf2::{Uf2Block, Uf2Error, Uf2Image, Uf2Segment};

use object::{
    Architecture as ObjectArchitecture, BinaryFormat, Endianness, Object, ObjectSegment,
    ObjectSymbol, SegmentFlags, SymbolKind, elf::PT_LOAD, read::elf::ProgramHeader,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Architecture encoded by a supported firmware ELF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirmwareArchitecture {
    /// 32-bit RISC-V.
    RiscV32,
    /// 32-bit Arm.
    Arm,
    /// 32-bit Xtensa.
    Xtensa,
    /// 8-bit AVR enhanced RISC architecture.
    Avr8,
    /// MSP430 CPUX with a 20-bit architectural address space.
    Msp430X,
    /// Enhanced mid-range PIC16 architecture.
    Pic16Enhanced,
    /// MCS-51/8051 architecture.
    Mcs51,
}

/// Loadable ELF segment including zero-initialized tail bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareSegment {
    /// Segment virtual address.
    pub address: u64,
    /// Distinct ELF physical/load address, when it differs from the virtual address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_address: Option<u64>,
    /// Complete memory image, padded to the ELF memory size.
    pub data: Vec<u8>,
    /// Segment permits instruction execution.
    pub executable: bool,
    /// Segment permits runtime writes.
    pub writable: bool,
    /// Requested ELF alignment.
    pub alignment: u64,
}

/// One defined symbol retained for debugging and test assertions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareSymbol {
    /// Symbol name.
    pub name: String,
    /// Virtual address.
    pub address: u64,
    /// Symbol size when known.
    pub size: u64,
    /// True for text symbols.
    pub code: bool,
}

/// Parsed firmware ELF.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareImage {
    /// Target CPU architecture.
    pub architecture: FirmwareArchitecture,
    /// ELF entry point.
    pub entry: u64,
    /// Loadable segments in address order.
    pub segments: Vec<FirmwareSegment>,
    /// Defined symbols in deterministic `(address, name)` order.
    pub symbols: Vec<FirmwareSymbol>,
}

impl FirmwareImage {
    /// Parses a little-endian, 32-bit ELF firmware image.
    pub fn parse(bytes: &[u8]) -> Result<Self, ImageError> {
        let file = object::File::parse(bytes).map_err(|error| ImageError::Parse {
            message: error.to_string(),
        })?;
        if file.format() != BinaryFormat::Elf {
            return Err(ImageError::UnsupportedFormat(format!(
                "{:?}",
                file.format()
            )));
        }
        if file.endianness() != Endianness::Little {
            return Err(ImageError::UnsupportedEndianness(format!(
                "{:?}",
                file.endianness()
            )));
        }
        let architecture = match file.architecture() {
            ObjectArchitecture::Riscv32 => FirmwareArchitecture::RiscV32,
            ObjectArchitecture::Arm => FirmwareArchitecture::Arm,
            ObjectArchitecture::Xtensa => FirmwareArchitecture::Xtensa,
            ObjectArchitecture::Avr => FirmwareArchitecture::Avr8,
            ObjectArchitecture::Msp430 => FirmwareArchitecture::Msp430X,
            architecture => {
                return Err(ImageError::UnsupportedArchitecture(format!(
                    "{architecture:?}"
                )));
            }
        };

        let physical_addresses = match &file {
            object::File::Elf32(elf) => elf
                .elf_program_headers()
                .iter()
                .filter(|header| header.p_type(elf.endian()) == PT_LOAD)
                .map(|header| u64::from(header.p_paddr(elf.endian())))
                .collect::<Vec<_>>(),
            object::File::Elf64(elf) => elf
                .elf_program_headers()
                .iter()
                .filter(|header| header.p_type(elf.endian()) == PT_LOAD)
                .map(|header| header.p_paddr(elf.endian()))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut segments = Vec::new();
        for (index, segment) in file.segments().enumerate() {
            if segment.size() == 0 {
                continue;
            }
            let memory_size =
                usize::try_from(segment.size()).map_err(|_| ImageError::SegmentTooLarge {
                    address: segment.address(),
                    size: segment.size(),
                })?;
            let source = segment.data().map_err(|error| ImageError::Parse {
                message: error.to_string(),
            })?;
            if source.len() > memory_size {
                return Err(ImageError::MalformedSegment {
                    address: segment.address(),
                    file_size: source.len() as u64,
                    memory_size: segment.size(),
                });
            }
            let mut data = vec![0; memory_size];
            data[..source.len()].copy_from_slice(source);
            let (executable, writable) = match segment.flags() {
                SegmentFlags::Elf { p_flags } => (
                    p_flags & object::elf::PF_X != 0,
                    p_flags & object::elf::PF_W != 0,
                ),
                _ => (false, false),
            };
            segments.push(FirmwareSegment {
                address: segment.address(),
                load_address: physical_addresses
                    .get(index)
                    .copied()
                    .filter(|address| *address != segment.address()),
                data,
                executable,
                writable,
                alignment: segment.align(),
            });
        }
        segments.sort_by_key(|segment| segment.address);
        if segments.is_empty() {
            return Err(ImageError::NoLoadableSegments);
        }

        let mut symbols = Vec::new();
        for symbol in file.symbols().chain(file.dynamic_symbols()) {
            if symbol.is_undefined() || symbol.address() == 0 {
                continue;
            }
            let Ok(name) = symbol.name() else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            symbols.push(FirmwareSymbol {
                name: name.to_owned(),
                address: symbol.address(),
                size: symbol.size(),
                code: symbol.kind() == SymbolKind::Text,
            });
        }
        symbols.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.name.cmp(&right.name))
        });
        symbols.dedup_by(|left, right| left.address == right.address && left.name == right.name);

        Ok(Self {
            architecture,
            entry: file.entry(),
            segments,
            symbols,
        })
    }

    /// Finds the closest symbol whose address is not greater than `address`.
    pub fn symbolicate(&self, address: u64) -> Option<(&FirmwareSymbol, u64)> {
        let index = self
            .symbols
            .partition_point(|symbol| symbol.address <= address)
            .checked_sub(1)?;
        let symbol = &self.symbols[index];
        Some((symbol, address - symbol.address))
    }
}

/// Firmware parsing failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ImageError {
    /// Generic object parser failure.
    #[error("ELF parse failed: {message}")]
    Parse {
        /// Parser diagnostic.
        message: String,
    },
    /// File is not ELF.
    #[error("unsupported firmware format {0}")]
    UnsupportedFormat(String),
    /// Target endianness is outside the initial portfolio.
    #[error("unsupported firmware endianness {0}")]
    UnsupportedEndianness(String),
    /// CPU architecture is outside the initial portfolio.
    #[error("unsupported firmware architecture {0}")]
    UnsupportedArchitecture(String),
    /// Segment allocation cannot fit on the host.
    #[error("segment at {address:#x} is too large ({size} bytes)")]
    SegmentTooLarge {
        /// Segment address.
        address: u64,
        /// Requested in-memory size.
        size: u64,
    },
    /// File-backed segment content exceeds its memory size.
    #[error(
        "segment at {address:#x} has file size {file_size} larger than memory size {memory_size}"
    )]
    MalformedSegment {
        /// Segment address.
        address: u64,
        /// File-backed byte count.
        file_size: u64,
        /// In-memory byte count.
        memory_size: u64,
    },
    /// ELF has no non-empty loadable program segments.
    #[error("ELF contains no loadable segments")]
    NoLoadableSegments,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_elf_input() {
        assert!(matches!(
            FirmwareImage::parse(b"not an elf"),
            Err(ImageError::Parse { .. })
        ));
    }

    #[test]
    fn parses_minimal_riscv32_elf() {
        let elf = minimal_riscv_elf();
        let image = FirmwareImage::parse(&elf).unwrap();
        assert_eq!(image.architecture, FirmwareArchitecture::RiscV32);
        assert_eq!(image.entry, 0x1000);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].address, 0x1000);
        assert_eq!(image.segments[0].data, vec![0x13, 0, 0, 0, 0, 0, 0, 0]);
        assert!(image.segments[0].executable);
        assert!(!image.segments[0].writable);
    }

    fn minimal_riscv_elf() -> Vec<u8> {
        // ELF32 little-endian with one PT_LOAD program header. The file holds a
        // single RISC-V NOP and requests four BSS bytes after it.
        let mut bytes = vec![0_u8; 0x104];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1; // ELFCLASS32
        bytes[5] = 1; // ELFDATA2LSB
        bytes[6] = 1; // EV_CURRENT
        put_u16(&mut bytes, 16, 2); // ET_EXEC
        put_u16(&mut bytes, 18, 243); // EM_RISCV
        put_u32(&mut bytes, 20, 1);
        put_u32(&mut bytes, 24, 0x1000);
        put_u32(&mut bytes, 28, 52);
        put_u16(&mut bytes, 40, 52);
        put_u16(&mut bytes, 42, 32);
        put_u16(&mut bytes, 44, 1);
        put_u32(&mut bytes, 52, 1); // PT_LOAD
        put_u32(&mut bytes, 56, 0x100);
        put_u32(&mut bytes, 60, 0x1000);
        put_u32(&mut bytes, 64, 0x1000);
        put_u32(&mut bytes, 68, 4);
        put_u32(&mut bytes, 72, 8);
        put_u32(&mut bytes, 76, object::elf::PF_R | object::elf::PF_X);
        put_u32(&mut bytes, 80, 4);
        bytes[0x100..0x104].copy_from_slice(&[0x13, 0, 0, 0]);
        bytes
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
