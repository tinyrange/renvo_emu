use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Intel HEX record classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelHexRecordType {
    /// Bytes loaded at the current extended base plus the record address.
    Data,
    /// Required terminal record.
    EndOfFile,
    /// Twenty-bit segment base (`value << 4`).
    ExtendedSegmentAddress,
    /// Start address encoded as CS:IP.
    StartSegmentAddress,
    /// Upper sixteen bits of a 32-bit linear address.
    ExtendedLinearAddress,
    /// 32-bit linear start address.
    StartLinearAddress,
}

/// One validated source record retained for provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelHexRecord {
    /// One-based source line number.
    pub line: usize,
    /// Sixteen-bit record-relative address.
    pub address: u16,
    /// Record classification.
    pub kind: IntelHexRecordType,
    /// Record payload, excluding the checksum.
    pub data: Vec<u8>,
    /// Source checksum byte.
    pub checksum: u8,
}

/// One contiguous byte-addressed region reconstructed from data records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelHexSegment {
    /// Absolute byte address.
    pub address: u32,
    /// Contiguous bytes beginning at `address`.
    pub data: Vec<u8>,
}

/// Strictly parsed Intel HEX image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelHexImage {
    /// Validated records in source order.
    pub records: Vec<IntelHexRecord>,
    /// Reconstructed non-overlapping byte regions in address order.
    pub segments: Vec<IntelHexSegment>,
    /// Optional start address supplied by a type 03 or type 05 record.
    pub entry: Option<u32>,
}

/// Byte order used to reconstruct non-byte-wide program words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramWordEndianness {
    /// Least-significant byte appears first in the HEX data.
    Little,
    /// Most-significant byte appears first in the HEX data.
    Big,
}

/// One contiguous word-addressed program region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramWordSegment {
    /// First architecture-visible program-word address.
    pub address: u32,
    /// Reconstructed words with unused high bits validated as zero.
    pub words: Vec<u32>,
}

/// Explicit word-addressed view of an Intel HEX image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramWordImage {
    /// Number of meaningful bits per word.
    pub word_bits: u8,
    /// Source byte order.
    pub endianness: ProgramWordEndianness,
    /// Reconstructed regions in program-word address order.
    pub segments: Vec<ProgramWordSegment>,
    /// Optional start address converted from bytes to words when aligned.
    pub entry: Option<u32>,
}

impl IntelHexImage {
    /// Parses and validates an ASCII Intel HEX document.
    pub fn parse(source: &[u8]) -> Result<Self, IntelHexError> {
        let text = std::str::from_utf8(source).map_err(|_| IntelHexError::NonAscii)?;
        let mut records = Vec::new();
        let mut bytes = BTreeMap::new();
        let mut base = 0_u32;
        let mut entry = None;
        let mut eof = false;

        for (index, raw_line) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            if line.is_empty() {
                return Err(IntelHexError::EmptyLine(line_number));
            }
            if eof {
                return Err(IntelHexError::AfterEndOfFile(line_number));
            }
            let encoded = line
                .strip_prefix(':')
                .ok_or(IntelHexError::MissingColon(line_number))?;
            let decoded = hex::decode(encoded).map_err(|_| IntelHexError::Hex(line_number))?;
            if decoded.len() < 5 {
                return Err(IntelHexError::Length(line_number));
            }
            let length = usize::from(decoded[0]);
            if decoded.len() != length + 5 {
                return Err(IntelHexError::Length(line_number));
            }
            if decoded
                .iter()
                .fold(0_u8, |sum, byte| sum.wrapping_add(*byte))
                != 0
            {
                return Err(IntelHexError::Checksum(line_number));
            }
            let address = u16::from_be_bytes([decoded[1], decoded[2]]);
            let data = decoded[4..4 + length].to_vec();
            let checksum = decoded[4 + length];
            let kind = match decoded[3] {
                0 => IntelHexRecordType::Data,
                1 => IntelHexRecordType::EndOfFile,
                2 => IntelHexRecordType::ExtendedSegmentAddress,
                3 => IntelHexRecordType::StartSegmentAddress,
                4 => IntelHexRecordType::ExtendedLinearAddress,
                5 => IntelHexRecordType::StartLinearAddress,
                other => {
                    return Err(IntelHexError::RecordType {
                        line: line_number,
                        other,
                    });
                }
            };

            match kind {
                IntelHexRecordType::Data => {
                    let start = base
                        .checked_add(u32::from(address))
                        .ok_or(IntelHexError::AddressOverflow(line_number))?;
                    for (offset, byte) in data.iter().copied().enumerate() {
                        let absolute = start
                            .checked_add(offset as u32)
                            .ok_or(IntelHexError::AddressOverflow(line_number))?;
                        if bytes.insert(absolute, byte).is_some() {
                            return Err(IntelHexError::Overlap {
                                line: line_number,
                                address: absolute,
                            });
                        }
                    }
                }
                IntelHexRecordType::EndOfFile => {
                    require_shape(line_number, address, &data, 0)?;
                    eof = true;
                }
                IntelHexRecordType::ExtendedSegmentAddress => {
                    require_shape(line_number, address, &data, 2)?;
                    base = u32::from(u16::from_be_bytes([data[0], data[1]])) << 4;
                }
                IntelHexRecordType::StartSegmentAddress => {
                    require_shape(line_number, address, &data, 4)?;
                    let segment = u32::from(u16::from_be_bytes([data[0], data[1]]));
                    let offset = u32::from(u16::from_be_bytes([data[2], data[3]]));
                    set_entry(&mut entry, (segment << 4) + offset, line_number)?;
                }
                IntelHexRecordType::ExtendedLinearAddress => {
                    require_shape(line_number, address, &data, 2)?;
                    base = u32::from(u16::from_be_bytes([data[0], data[1]])) << 16;
                }
                IntelHexRecordType::StartLinearAddress => {
                    require_shape(line_number, address, &data, 4)?;
                    set_entry(
                        &mut entry,
                        u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
                        line_number,
                    )?;
                }
            }
            records.push(IntelHexRecord {
                line: line_number,
                address,
                kind,
                data,
                checksum,
            });
        }
        if !eof {
            return Err(IntelHexError::MissingEndOfFile);
        }

        Ok(Self {
            records,
            segments: coalesce(bytes),
            entry,
        })
    }

    /// Converts byte regions into explicitly word-addressed program storage.
    pub fn program_words(
        &self,
        word_bits: u8,
        endianness: ProgramWordEndianness,
    ) -> Result<ProgramWordImage, IntelHexError> {
        if !(1..=32).contains(&word_bits) {
            return Err(IntelHexError::WordBits(word_bits));
        }
        let word_bytes = usize::from(word_bits.div_ceil(8));
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in &self.segments {
            if segment.address % word_bytes as u32 != 0 || segment.data.len() % word_bytes != 0 {
                return Err(IntelHexError::WordAlignment {
                    address: segment.address,
                    length: segment.data.len(),
                    word_bytes,
                });
            }
            let mut words = Vec::with_capacity(segment.data.len() / word_bytes);
            for (index, chunk) in segment.data.chunks_exact(word_bytes).enumerate() {
                let mut value = 0_u32;
                match endianness {
                    ProgramWordEndianness::Little => {
                        for (shift, byte) in chunk.iter().copied().enumerate() {
                            value |= u32::from(byte) << (shift * 8);
                        }
                    }
                    ProgramWordEndianness::Big => {
                        for byte in chunk.iter().copied() {
                            value = (value << 8) | u32::from(byte);
                        }
                    }
                }
                if word_bits < 32 && value >= (1_u32 << word_bits) {
                    return Err(IntelHexError::WordRange {
                        address: segment.address + (index * word_bytes) as u32,
                        value,
                        word_bits,
                    });
                }
                words.push(value);
            }
            segments.push(ProgramWordSegment {
                address: segment.address / word_bytes as u32,
                words,
            });
        }
        let entry = self
            .entry
            .map(|entry| {
                if entry % word_bytes as u32 == 0 {
                    Ok(entry / word_bytes as u32)
                } else {
                    Err(IntelHexError::EntryAlignment { entry, word_bytes })
                }
            })
            .transpose()?;
        Ok(ProgramWordImage {
            word_bits,
            endianness,
            segments,
            entry,
        })
    }
}

fn require_shape(
    line: usize,
    address: u16,
    data: &[u8],
    length: usize,
) -> Result<(), IntelHexError> {
    if address != 0 || data.len() != length {
        return Err(IntelHexError::RecordShape(line));
    }
    Ok(())
}

fn set_entry(entry: &mut Option<u32>, value: u32, line: usize) -> Result<(), IntelHexError> {
    if entry.is_some_and(|existing| existing != value) {
        return Err(IntelHexError::ConflictingEntry(line));
    }
    *entry = Some(value);
    Ok(())
}

fn coalesce(bytes: BTreeMap<u32, u8>) -> Vec<IntelHexSegment> {
    let mut segments: Vec<IntelHexSegment> = Vec::new();
    for (address, byte) in bytes {
        if let Some(segment) = segments.last_mut()
            && segment.address + segment.data.len() as u32 == address
        {
            segment.data.push(byte);
        } else {
            segments.push(IntelHexSegment {
                address,
                data: vec![byte],
            });
        }
    }
    segments
}

/// Intel HEX validation or reconstruction failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IntelHexError {
    /// Input was not UTF-8 ASCII-compatible text.
    #[error("Intel HEX input is not ASCII text")]
    NonAscii,
    /// Blank records are rejected so provenance remains unambiguous.
    #[error("Intel HEX line {0} is empty")]
    EmptyLine(usize),
    /// A record did not begin with a colon.
    #[error("Intel HEX line {0} does not begin with ':'")]
    MissingColon(usize),
    /// A record contained malformed hexadecimal text.
    #[error("Intel HEX line {0} contains malformed hexadecimal text")]
    Hex(usize),
    /// Byte count did not match encoded record length.
    #[error("Intel HEX line {0} has an invalid encoded length")]
    Length(usize),
    /// Record checksum did not sum to zero.
    #[error("Intel HEX line {0} has an invalid checksum")]
    Checksum(usize),
    /// Record type is not part of the Intel HEX specification used by Renvo Emulator.
    #[error("Intel HEX line {line} uses unsupported record type {other:#04x}")]
    RecordType { line: usize, other: u8 },
    /// A control record had the wrong address or payload size.
    #[error("Intel HEX line {0} has an invalid control-record shape")]
    RecordShape(usize),
    /// A record appeared after EOF.
    #[error("Intel HEX line {0} appears after the EOF record")]
    AfterEndOfFile(usize),
    /// EOF was absent.
    #[error("Intel HEX input has no EOF record")]
    MissingEndOfFile,
    /// Extended and relative addresses overflowed 32 bits.
    #[error("Intel HEX address overflows at line {0}")]
    AddressOverflow(usize),
    /// Two data records attempted to define the same byte.
    #[error("Intel HEX line {line} overlaps byte address {address:#010x}")]
    Overlap { line: usize, address: u32 },
    /// Multiple start records disagreed.
    #[error("Intel HEX line {0} conflicts with an earlier start address")]
    ConflictingEntry(usize),
    /// Program-word width is invalid.
    #[error("program-word width {0} is outside 1..=32 bits")]
    WordBits(u8),
    /// A byte segment cannot be divided into complete aligned words.
    #[error(
        "byte segment at {address:#010x} with length {length} is not aligned to {word_bytes}-byte words"
    )]
    WordAlignment {
        address: u32,
        length: usize,
        word_bytes: usize,
    },
    /// A reconstructed word sets bits outside the architecture width.
    #[error("program word {value:#010x} at byte address {address:#010x} exceeds {word_bits} bits")]
    WordRange {
        address: u32,
        value: u32,
        word_bits: u8,
    },
    /// Start address is not aligned to a program word.
    #[error("entry {entry:#010x} is not aligned to {word_bytes}-byte program words")]
    EntryAlignment { entry: u32, word_bytes: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_linear_records_and_retains_source() {
        let source = b":020000040001F9\n:0400100001020304E2\n:0400000500010010E6\n:00000001FF\n";
        let image = IntelHexImage::parse(source).unwrap();
        assert_eq!(image.entry, Some(0x0001_0010));
        assert_eq!(image.records.len(), 4);
        assert_eq!(
            image.segments,
            vec![IntelHexSegment {
                address: 0x0001_0010,
                data: vec![1, 2, 3, 4],
            }]
        );
    }

    #[test]
    fn rejects_checksum_overlap_and_records_after_eof() {
        assert_eq!(
            IntelHexImage::parse(b":0100000001FF\n:00000001FF\n"),
            Err(IntelHexError::Checksum(1))
        );
        assert!(matches!(
            IntelHexImage::parse(b":020000000102FB\n:0100010003FB\n:00000001FF\n"),
            Err(IntelHexError::Overlap { address: 1, .. })
        ));
        assert_eq!(
            IntelHexImage::parse(b":00000001FF\n:00000001FF\n"),
            Err(IntelHexError::AfterEndOfFile(2))
        );
    }

    #[test]
    fn reconstructs_pic_style_fourteen_bit_words() {
        let image = IntelHexImage::parse(b":040000003412FF3F78\n:00000001FF\n").unwrap();
        let words = image
            .program_words(14, ProgramWordEndianness::Little)
            .unwrap();
        assert_eq!(words.segments[0].address, 0);
        assert_eq!(words.segments[0].words, [0x1234, 0x3fff]);
    }

    #[test]
    fn rejects_bits_outside_program_word_width() {
        let image = IntelHexImage::parse(b":0200000000C03E\n:00000001FF\n").unwrap();
        assert!(matches!(
            image.program_words(14, ProgramWordEndianness::Little),
            Err(IntelHexError::WordRange { .. })
        ));
    }
}
