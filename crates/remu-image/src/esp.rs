use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const IMAGE_MAGIC: u8 = 0xe9;
const IMAGE_HEADER_SIZE: usize = 24;
const SEGMENT_HEADER_SIZE: usize = 8;
const CHECKSUM_MAGIC: u8 = 0xef;
const PARTITION_OFFSET: usize = 0x8000;
const PARTITION_TABLE_SIZE: usize = 0x1000;
const PARTITION_ENTRY_SIZE: usize = 32;
const PARTITION_MAGIC: u16 = 0x50aa;
const PARTITION_MD5_MAGIC: u16 = 0xebeb;

/// ESP ROM-loadable image header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspImageHeader {
    /// Number of load/map segments.
    pub segment_count: u8,
    /// SPI flash mode field.
    pub flash_mode: u8,
    /// Packed flash-size/frequency field.
    pub flash_size_frequency: u8,
    /// CPU entry point.
    pub entry: u32,
    /// SPI write-protect pin field.
    pub write_protect_pin: u8,
    /// Packed SPI pin drive settings.
    pub drive_settings: [u8; 3],
    /// Espressif chip identifier.
    pub chip_id: u16,
    /// Deprecated minimum revision byte.
    pub minimum_revision_legacy: u8,
    /// Full minimum chip revision.
    pub minimum_revision: u16,
    /// Full maximum chip revision.
    pub maximum_revision: u16,
    /// True when a simple SHA-256 follows the checksum.
    pub hash_appended: bool,
}

/// One segment from an ESP executable image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspImageSegment {
    /// Destination virtual/load address.
    pub address: u32,
    /// Segment data offset in the complete flash image.
    pub flash_offset: u32,
    /// Segment bytes.
    pub data: Vec<u8>,
}

/// Validated ESP bootloader or application image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspExecutableImage {
    /// Absolute flash offset at which the image header begins.
    pub flash_offset: u32,
    /// Parsed header.
    pub header: EspImageHeader,
    /// Load/map segments.
    pub segments: Vec<EspImageSegment>,
    /// Stored and validated XOR checksum.
    pub checksum: u8,
    /// Stored and validated SHA-256, when present.
    pub appended_sha256: Option<String>,
    /// First byte after checksum and optional hash.
    pub end_offset: u32,
}

/// One ESP partition-table entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspPartition {
    /// Partition type.
    pub partition_type: u8,
    /// Partition subtype.
    pub subtype: u8,
    /// Flash byte offset.
    pub offset: u32,
    /// Declared byte size.
    pub size: u32,
    /// NUL-terminated UTF-8 label.
    pub label: String,
    /// Partition flags.
    pub flags: u32,
}

/// Validated ESP partition table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspPartitionTable {
    /// Parsed entries in table order.
    pub partitions: Vec<EspPartition>,
    /// True when an ESP-IDF MD5 trailer entry was present.
    pub has_md5: bool,
}

/// ESP merged flash image containing bootloader, partition table and application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EspFlashImage {
    /// ROM-loaded second-stage bootloader.
    pub bootloader: EspExecutableImage,
    /// Flash partition table.
    pub partition_table: EspPartitionTable,
    /// Factory or first application image.
    pub application: EspExecutableImage,
    /// Selected application partition.
    pub application_partition: EspPartition,
}

impl EspExecutableImage {
    /// Parses an executable image beginning at byte zero.
    pub fn parse(bytes: &[u8]) -> Result<Self, EspImageError> {
        Self::parse_at(bytes, 0, bytes.len())
    }

    fn parse_at(bytes: &[u8], start: usize, limit: usize) -> Result<Self, EspImageError> {
        if start >= limit || limit > bytes.len() || limit - start < IMAGE_HEADER_SIZE {
            return Err(EspImageError::Truncated {
                offset: start,
                needed: IMAGE_HEADER_SIZE,
                available: limit.saturating_sub(start),
            });
        }
        if bytes[start] != IMAGE_MAGIC {
            return Err(EspImageError::Magic {
                offset: start,
                actual: bytes[start],
            });
        }
        let header = EspImageHeader {
            segment_count: bytes[start + 1],
            flash_mode: bytes[start + 2],
            flash_size_frequency: bytes[start + 3],
            entry: read_u32(bytes, start + 4),
            write_protect_pin: bytes[start + 8],
            drive_settings: bytes[start + 9..start + 12]
                .try_into()
                .expect("fixed ESP header field"),
            chip_id: read_u16(bytes, start + 12),
            minimum_revision_legacy: bytes[start + 14],
            minimum_revision: read_u16(bytes, start + 15),
            maximum_revision: read_u16(bytes, start + 17),
            hash_appended: bytes[start + 23] == 1,
        };
        if header.segment_count == 0 || header.segment_count > 16 {
            return Err(EspImageError::SegmentCount(header.segment_count));
        }

        let mut cursor = start + IMAGE_HEADER_SIZE;
        let mut checksum = CHECKSUM_MAGIC;
        let mut segments = Vec::with_capacity(usize::from(header.segment_count));
        for index in 0..header.segment_count {
            require(bytes, cursor, SEGMENT_HEADER_SIZE, limit)?;
            let address = read_u32(bytes, cursor);
            let length =
                usize::try_from(read_u32(bytes, cursor + 4)).expect("ESP segment length fits host");
            cursor += SEGMENT_HEADER_SIZE;
            require(bytes, cursor, length, limit)?;
            let segment_bytes = bytes[cursor..cursor + length].to_vec();
            for byte in &segment_bytes {
                checksum ^= byte;
            }
            segments.push(EspImageSegment {
                address,
                flash_offset: u32::try_from(cursor)
                    .map_err(|_| EspImageError::OffsetTooLarge(cursor))?,
                data: segment_bytes,
            });
            cursor += length;
            if cursor > limit {
                return Err(EspImageError::SegmentBounds { index, limit });
            }
        }

        let checksum_offset = cursor
            .checked_div(16)
            .and_then(|group| group.checked_add(1))
            .and_then(|group| group.checked_mul(16))
            .and_then(|end| end.checked_sub(1))
            .ok_or(EspImageError::OffsetTooLarge(cursor))?;
        require(bytes, cursor, checksum_offset - cursor + 1, limit)?;
        if bytes[cursor..checksum_offset]
            .iter()
            .any(|padding| *padding != 0)
        {
            return Err(EspImageError::Padding { offset: cursor });
        }
        let stored_checksum = bytes[checksum_offset];
        if checksum != stored_checksum {
            return Err(EspImageError::Checksum {
                expected: checksum,
                actual: stored_checksum,
            });
        }
        cursor = checksum_offset + 1;

        let appended_sha256 = if header.hash_appended {
            require(bytes, cursor, 32, limit)?;
            let expected = &bytes[cursor..cursor + 32];
            let actual = Sha256::digest(&bytes[start..cursor]);
            if actual.as_slice() != expected {
                return Err(EspImageError::Sha256 {
                    expected: hex::encode(expected),
                    actual: hex::encode(actual),
                });
            }
            cursor += 32;
            Some(hex::encode(expected))
        } else {
            None
        };

        Ok(Self {
            flash_offset: u32::try_from(start).map_err(|_| EspImageError::OffsetTooLarge(start))?,
            header,
            segments,
            checksum: stored_checksum,
            appended_sha256,
            end_offset: u32::try_from(cursor).map_err(|_| EspImageError::OffsetTooLarge(cursor))?,
        })
    }
}

impl EspPartitionTable {
    /// Parses the standard 4 KiB partition-table sector at flash offset `0x8000`.
    pub fn parse(flash: &[u8]) -> Result<Self, EspImageError> {
        require(flash, PARTITION_OFFSET, PARTITION_TABLE_SIZE, flash.len())?;
        let table = &flash[PARTITION_OFFSET..PARTITION_OFFSET + PARTITION_TABLE_SIZE];
        let mut partitions = Vec::new();
        let mut has_md5 = false;
        for (index, entry) in table.chunks_exact(PARTITION_ENTRY_SIZE).enumerate() {
            let magic = read_u16(entry, 0);
            if magic == u16::MAX {
                break;
            }
            if magic == PARTITION_MD5_MAGIC {
                has_md5 = true;
                break;
            }
            if magic != PARTITION_MAGIC {
                return Err(EspImageError::PartitionMagic { index, magic });
            }
            let label_end = entry[12..28]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(16);
            let label = std::str::from_utf8(&entry[12..12 + label_end])
                .map_err(|_| EspImageError::PartitionLabel(index))?
                .to_owned();
            partitions.push(EspPartition {
                partition_type: entry[2],
                subtype: entry[3],
                offset: read_u32(entry, 4),
                size: read_u32(entry, 8),
                label,
                flags: read_u32(entry, 28),
            });
        }
        if partitions.is_empty() {
            return Err(EspImageError::NoPartitions);
        }

        let mut ordered = partitions.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|partition| partition.offset);
        for pair in ordered.windows(2) {
            let end = pair[0]
                .offset
                .checked_add(pair[0].size)
                .ok_or(EspImageError::PartitionOverflow(pair[0].label.clone()))?;
            if end > pair[1].offset {
                return Err(EspImageError::PartitionOverlap {
                    first: pair[0].label.clone(),
                    second: pair[1].label.clone(),
                });
            }
        }

        Ok(Self {
            partitions,
            has_md5,
        })
    }
}

impl EspFlashImage {
    /// Parses a standard merged ESP-IDF flash binary.
    pub fn parse(bytes: &[u8]) -> Result<Self, EspImageError> {
        let bootloader = EspExecutableImage::parse_at(bytes, 0, PARTITION_OFFSET)?;
        let partition_table = EspPartitionTable::parse(bytes)?;
        let application_partition = partition_table
            .partitions
            .iter()
            .find(|partition| partition.partition_type == 0 && partition.subtype == 0)
            .or_else(|| {
                partition_table
                    .partitions
                    .iter()
                    .find(|partition| partition.partition_type == 0)
            })
            .cloned()
            .ok_or(EspImageError::NoApplication)?;
        let start = usize::try_from(application_partition.offset)
            .expect("32-bit partition offset fits host");
        let declared_end = application_partition
            .offset
            .checked_add(application_partition.size)
            .ok_or_else(|| EspImageError::PartitionOverflow(application_partition.label.clone()))?;
        let limit = usize::try_from(declared_end)
            .expect("32-bit partition end fits host")
            .min(bytes.len());
        let application = EspExecutableImage::parse_at(bytes, start, limit)?;
        if application.end_offset > declared_end {
            return Err(EspImageError::ApplicationTooLarge {
                end: application.end_offset,
                partition_end: declared_end,
            });
        }
        Ok(Self {
            bootloader,
            partition_table,
            application,
            application_partition,
        })
    }
}

/// ESP image or partition validation failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EspImageError {
    #[error("ESP image at {offset:#x} needs {needed} bytes but only {available} remain")]
    Truncated {
        offset: usize,
        needed: usize,
        available: usize,
    },
    #[error("ESP image at {offset:#x} has magic {actual:#04x}, expected 0xe9")]
    Magic { offset: usize, actual: u8 },
    #[error("ESP image declares invalid segment count {0}")]
    SegmentCount(u8),
    #[error("ESP image segment {index} exceeds parse limit {limit:#x}")]
    SegmentBounds { index: u8, limit: usize },
    #[error("ESP image offset {0:#x} cannot fit the format")]
    OffsetTooLarge(usize),
    #[error("ESP image has non-zero footer padding at {offset:#x}")]
    Padding { offset: usize },
    #[error("ESP checksum mismatch: expected {expected:#04x}, got {actual:#04x}")]
    Checksum { expected: u8, actual: u8 },
    #[error("ESP appended SHA-256 mismatch: expected {expected}, got {actual}")]
    Sha256 { expected: String, actual: String },
    #[error("ESP partition entry {index} has invalid magic {magic:#06x}")]
    PartitionMagic { index: usize, magic: u16 },
    #[error("ESP partition entry {0} has a non-UTF-8 label")]
    PartitionLabel(usize),
    #[error("ESP partition table has no data entries")]
    NoPartitions,
    #[error("ESP partition {0:?} address range overflows")]
    PartitionOverflow(String),
    #[error("ESP partitions {first:?} and {second:?} overlap")]
    PartitionOverlap { first: String, second: String },
    #[error("ESP partition table has no application partition")]
    NoApplication,
    #[error("ESP application ends at {end:#x}, beyond declared partition end {partition_end:#x}")]
    ApplicationTooLarge { end: u32, partition_end: u32 },
}

fn require(bytes: &[u8], offset: usize, length: usize, limit: usize) -> Result<(), EspImageError> {
    let end = offset
        .checked_add(length)
        .ok_or(EspImageError::OffsetTooLarge(offset))?;
    if end > limit || end > bytes.len() {
        return Err(EspImageError::Truncated {
            offset,
            needed: length,
            available: limit.min(bytes.len()).saturating_sub(offset),
        });
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated ESP u16 field"),
    )
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated ESP u32 field"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_an_executable_image() {
        let bytes = executable_image(9, 0x4037_5414, 0x3fce_2820, &[1, 2, 3, 4]);
        let image = EspExecutableImage::parse(&bytes).unwrap();
        assert_eq!(image.header.chip_id, 9);
        assert_eq!(image.header.entry, 0x4037_5414);
        assert_eq!(image.segments[0].data, [1, 2, 3, 4]);
        assert!(image.appended_sha256.is_some());
    }

    #[test]
    fn rejects_a_corrupted_segment() {
        let mut bytes = executable_image(13, 0x4080_02e6, 0x4087_5730, &[1, 2, 3, 4]);
        bytes[IMAGE_HEADER_SIZE + SEGMENT_HEADER_SIZE] ^= 1;
        assert!(matches!(
            EspExecutableImage::parse(&bytes),
            Err(EspImageError::Checksum { .. })
        ));
    }

    fn executable_image(chip: u16, entry: u32, address: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; IMAGE_HEADER_SIZE];
        bytes[0] = IMAGE_MAGIC;
        bytes[1] = 1;
        bytes[4..8].copy_from_slice(&entry.to_le_bytes());
        bytes[8] = 0xee;
        bytes[12..14].copy_from_slice(&chip.to_le_bytes());
        bytes[23] = 1;
        bytes.extend(address.to_le_bytes());
        bytes.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend(data);
        let checksum_offset = ((bytes.len() / 16) + 1) * 16 - 1;
        bytes.resize(checksum_offset, 0);
        let checksum = data.iter().fold(CHECKSUM_MAGIC, |sum, byte| sum ^ byte);
        bytes.push(checksum);
        let hash = Sha256::digest(&bytes);
        bytes.extend(hash);
        bytes
    }
}
