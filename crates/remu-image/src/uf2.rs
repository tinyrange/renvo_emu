use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

const BLOCK_SIZE: usize = 512;
const DATA_OFFSET: usize = 32;
const DATA_CAPACITY: usize = 476;
const MAGIC_START_0: u32 = 0x0a32_4655;
const MAGIC_START_1: u32 = 0x9e5d_5157;
const MAGIC_END: u32 = 0x0ab1_6f30;
const FLAG_NOT_MAIN_FLASH: u32 = 0x0000_0001;
const FLAG_FILE_CONTAINER: u32 = 0x0000_1000;
const FLAG_FAMILY_ID: u32 = 0x0000_2000;
const FLAG_EXTENSION_TAGS: u32 = 0x0000_8000;

/// One validated UF2 block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uf2Block {
    /// Raw UF2 flag word.
    pub flags: u32,
    /// Destination address declared by the block.
    pub target_address: u32,
    /// Zero-based block number.
    pub block_number: u32,
    /// Total block count declared by the image.
    pub block_count: u32,
    /// Optional UF2 family identifier.
    pub family_id: Option<u32>,
    /// True when the block carries UF2 extension tags rather than normal flash payload.
    pub extension_tags: bool,
    /// Bytes carried by this block.
    pub data: Vec<u8>,
}

/// One contiguous address range reconstructed from UF2 blocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uf2Segment {
    /// First destination address.
    pub address: u32,
    /// Contiguous payload bytes.
    pub data: Vec<u8>,
    /// True when any contributing block is marked as not-main-flash.
    pub not_main_flash: bool,
}

/// A complete, strictly validated UF2 image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uf2Image {
    /// Consistent family identifier, when present.
    pub family_id: Option<u32>,
    /// Blocks in block-number order.
    pub blocks: Vec<Uf2Block>,
    /// Extension-tag metadata blocks in physical file order.
    pub metadata_blocks: Vec<Uf2Block>,
    /// Address-ordered, non-overlapping reconstructed segments.
    pub segments: Vec<Uf2Segment>,
}

impl Uf2Image {
    /// Parses a complete UF2 file and rejects missing, duplicate, overlapping, or malformed blocks.
    pub fn parse(bytes: &[u8]) -> Result<Self, Uf2Error> {
        if bytes.is_empty() || bytes.len() % BLOCK_SIZE != 0 {
            return Err(Uf2Error::InvalidLength(bytes.len()));
        }

        let physical_count = u32::try_from(bytes.len() / BLOCK_SIZE)
            .map_err(|_| Uf2Error::TooManyBlocks(bytes.len() / BLOCK_SIZE))?;
        let mut blocks = BTreeMap::new();
        let mut metadata_blocks = Vec::new();
        let mut declared_count = None;
        let mut family_id = None;

        for (physical_index, raw) in bytes.chunks_exact(BLOCK_SIZE).enumerate() {
            let physical_index =
                u32::try_from(physical_index).expect("physical UF2 index fits validated count");
            if read_u32(raw, 0) != MAGIC_START_0
                || read_u32(raw, 4) != MAGIC_START_1
                || read_u32(raw, 508) != MAGIC_END
            {
                return Err(Uf2Error::BadMagic(physical_index));
            }

            let flags = read_u32(raw, 8);
            if flags & FLAG_FILE_CONTAINER != 0 {
                return Err(Uf2Error::FileContainer(physical_index));
            }
            let target_address = read_u32(raw, 12);
            let payload_size = read_u32(raw, 16);
            let block_number = read_u32(raw, 20);
            let block_count = read_u32(raw, 24);
            if block_count == 0 || block_number >= block_count {
                return Err(Uf2Error::BlockNumber {
                    physical_index,
                    block_number,
                    block_count,
                });
            }
            let extension_tags = flags & FLAG_EXTENSION_TAGS != 0;
            if !extension_tags {
                if let Some(expected) = declared_count.replace(block_count)
                    && expected != block_count
                {
                    return Err(Uf2Error::InconsistentBlockCount {
                        expected,
                        actual: block_count,
                        physical_index,
                    });
                }
            }

            let payload_size =
                usize::try_from(payload_size).expect("UF2 payload size fits host usize");
            if payload_size == 0 || payload_size > DATA_CAPACITY {
                return Err(Uf2Error::PayloadSize {
                    physical_index,
                    size: payload_size,
                });
            }
            target_address
                .checked_add(u32::try_from(payload_size).expect("validated UF2 payload fits u32"))
                .ok_or(Uf2Error::AddressOverflow(physical_index))?;

            let block_family = (flags & FLAG_FAMILY_ID != 0).then(|| read_u32(raw, 28));
            if !extension_tags {
                if let Some(value) = block_family {
                    if let Some(expected) = family_id
                        && expected != value
                    {
                        return Err(Uf2Error::InconsistentFamily {
                            expected,
                            actual: value,
                            physical_index,
                        });
                    }
                    family_id = Some(value);
                } else if family_id.is_some() {
                    return Err(Uf2Error::MissingFamily(physical_index));
                }
            }

            let block = Uf2Block {
                flags,
                target_address,
                block_number,
                block_count,
                family_id: block_family,
                extension_tags,
                data: raw[DATA_OFFSET..DATA_OFFSET + payload_size].to_vec(),
            };
            if extension_tags {
                metadata_blocks.push(block);
            } else if blocks.insert(block_number, block).is_some() {
                return Err(Uf2Error::DuplicateBlock(block_number));
            }
        }

        let declared_count = declared_count.ok_or(Uf2Error::NoFlashBlocks)?;
        if blocks.len() != usize::try_from(declared_count).expect("block count fits usize") {
            return Err(Uf2Error::MissingBlocks);
        }
        let actual_physical = blocks
            .len()
            .checked_add(metadata_blocks.len())
            .expect("validated UF2 block count fits host");
        if actual_physical != usize::try_from(physical_count).expect("physical block count fits") {
            return Err(Uf2Error::BlockCount {
                physical: physical_count,
                declared: declared_count,
            });
        }
        let blocks = blocks.into_values().collect::<Vec<_>>();
        let mut address_order = blocks.iter().collect::<Vec<_>>();
        address_order.sort_by_key(|block| block.target_address);

        let mut segments: Vec<Uf2Segment> = Vec::new();
        for block in address_order {
            let block_end = block
                .target_address
                .checked_add(u32::try_from(block.data.len()).expect("UF2 payload fits u32"))
                .ok_or(Uf2Error::AddressOverflow(block.block_number))?;
            if let Some(segment) = segments.last_mut() {
                let segment_end = segment
                    .address
                    .checked_add(
                        u32::try_from(segment.data.len())
                            .map_err(|_| Uf2Error::SegmentTooLarge(segment.data.len()))?,
                    )
                    .ok_or(Uf2Error::SegmentTooLarge(segment.data.len()))?;
                if block.target_address < segment_end {
                    return Err(Uf2Error::Overlap {
                        previous_end: segment_end,
                        next_start: block.target_address,
                    });
                }
                let not_main_flash = block.flags & FLAG_NOT_MAIN_FLASH != 0;
                if block.target_address == segment_end && not_main_flash == segment.not_main_flash {
                    segment.data.extend_from_slice(&block.data);
                    continue;
                }
            }
            let _ = block_end;
            segments.push(Uf2Segment {
                address: block.target_address,
                data: block.data.clone(),
                not_main_flash: block.flags & FLAG_NOT_MAIN_FLASH != 0,
            });
        }

        Ok(Self {
            family_id,
            blocks,
            metadata_blocks,
            segments,
        })
    }

    /// Reconstructs a fixed-size flash array rooted at `base`, filling unwritten bytes with
    /// `erased`.
    pub fn materialize(&self, base: u32, size: usize, erased: u8) -> Result<Vec<u8>, Uf2Error> {
        let size_u64 = u64::try_from(size).map_err(|_| Uf2Error::FlashTooLarge { base, size })?;
        let end = u64::from(base)
            .checked_add(size_u64)
            .ok_or(Uf2Error::FlashTooLarge { base, size })?;
        let mut flash = vec![erased; size];

        for segment in &self.segments {
            let segment_start = u64::from(segment.address);
            let segment_end = segment_start
                .checked_add(
                    u64::try_from(segment.data.len())
                        .map_err(|_| Uf2Error::SegmentTooLarge(segment.data.len()))?,
                )
                .ok_or(Uf2Error::SegmentTooLarge(segment.data.len()))?;
            if segment_start < u64::from(base) || segment_end > end {
                return Err(Uf2Error::OutsideFlash {
                    address: segment.address,
                    length: segment.data.len(),
                    base,
                    size,
                });
            }
            let offset =
                usize::try_from(segment_start - u64::from(base)).expect("validated offset fits");
            flash[offset..offset + segment.data.len()].copy_from_slice(&segment.data);
        }
        Ok(flash)
    }
}

/// UF2 validation failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Uf2Error {
    /// The input is not a non-empty sequence of 512-byte blocks.
    #[error("UF2 length {0} is not a non-zero multiple of 512 bytes")]
    InvalidLength(usize),
    /// The physical block count cannot be represented by UF2.
    #[error("UF2 contains too many blocks for its 32-bit format: {0}")]
    TooManyBlocks(usize),
    /// A block does not contain all three UF2 magic values.
    #[error("UF2 block {0} has invalid magic")]
    BadMagic(u32),
    /// The file-container mode is not a programmable flash image.
    #[error("UF2 file-container block {0} is not a flash image")]
    FileContainer(u32),
    /// A physical block has an invalid logical position.
    #[error("UF2 physical block {physical_index} declares block {block_number} of {block_count}")]
    BlockNumber {
        /// Zero-based position in the input file.
        physical_index: u32,
        /// Logical block number declared in the header.
        block_number: u32,
        /// Logical block count declared in the header.
        block_count: u32,
    },
    /// The physical and declared logical block counts disagree.
    #[error("UF2 has {physical} physical blocks but declares {declared}")]
    BlockCount {
        /// Physical block count.
        physical: u32,
        /// Declared logical block count.
        declared: u32,
    },
    /// Logical blocks disagree about their total count.
    #[error("UF2 block {physical_index} changes declared block count from {expected} to {actual}")]
    InconsistentBlockCount {
        /// First declared count.
        expected: u32,
        /// Conflicting declared count.
        actual: u32,
        /// Physical block containing the conflict.
        physical_index: u32,
    },
    /// A block carries no data or more than the UF2 payload capacity.
    #[error("UF2 block {physical_index} has invalid payload size {size}")]
    PayloadSize {
        /// Physical block containing the bad size.
        physical_index: u32,
        /// Declared byte count.
        size: usize,
    },
    /// A target address and payload length overflow 32 bits.
    #[error("UF2 block {0} destination address overflows")]
    AddressOverflow(u32),
    /// Logical blocks disagree about the target family.
    #[error(
        "UF2 block {physical_index} changes family identifier from {expected:#010x} to {actual:#010x}"
    )]
    InconsistentFamily {
        /// First family identifier.
        expected: u32,
        /// Conflicting family identifier.
        actual: u32,
        /// Physical block containing the conflict.
        physical_index: u32,
    },
    /// A block omits the family identifier used by the image.
    #[error("UF2 block {0} omits the family identifier used by earlier blocks")]
    MissingFamily(u32),
    /// A logical block number is duplicated.
    #[error("UF2 block number {0} appears more than once")]
    DuplicateBlock(u32),
    /// One or more logical blocks are absent.
    #[error("UF2 block sequence has missing block numbers")]
    MissingBlocks,
    /// The input only contained metadata blocks.
    #[error("UF2 contains no normal flash blocks")]
    NoFlashBlocks,
    /// Reconstructed payload ranges overlap.
    #[error(
        "UF2 address ranges overlap: previous end {previous_end:#x}, next start {next_start:#x}"
    )]
    Overlap {
        /// End of the previous range, exclusive.
        previous_end: u32,
        /// Start of the next range.
        next_start: u32,
    },
    /// A reconstructed segment cannot be represented on the host.
    #[error("UF2 reconstructed segment is too large: {0} bytes")]
    SegmentTooLarge(usize),
    /// The requested materialized flash address range is not representable.
    #[error("flash range rooted at {base:#010x} with {size} bytes overflows")]
    FlashTooLarge {
        /// First address in the flash mapping.
        base: u32,
        /// Requested byte length.
        size: usize,
    },
    /// A reconstructed segment is outside the selected flash mapping.
    #[error(
        "UF2 segment at {address:#010x} with {length} bytes is outside flash at {base:#010x} with {size} bytes"
    )]
    OutsideFlash {
        /// First address of the segment.
        address: u32,
        /// Segment byte length.
        length: usize,
        /// First address in the flash mapping.
        base: u32,
        /// Flash byte length.
        size: usize,
    },
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("UF2 fixed-width field is in range"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_coalesces_out_of_order_blocks() {
        let first = block(0, 2, 0x1000, &[1, 2, 3, 4], 0xe48b_ff56);
        let second = block(1, 2, 0x1004, &[5, 6, 7, 8], 0xe48b_ff56);
        let mut input = second;
        input.extend(first);
        let image = Uf2Image::parse(&input).unwrap();
        assert_eq!(image.family_id, Some(0xe48b_ff56));
        assert_eq!(image.blocks[0].block_number, 0);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].address, 0x1000);
        assert_eq!(image.segments[0].data, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rejects_overlapping_payloads() {
        let mut input = block(0, 2, 0x1000, &[1, 2, 3, 4], 1);
        input.extend(block(1, 2, 0x1002, &[5, 6, 7, 8], 1));
        assert!(matches!(
            Uf2Image::parse(&input),
            Err(Uf2Error::Overlap { .. })
        ));
    }

    #[test]
    fn materializes_at_the_requested_flash_base() {
        let input = block(0, 1, 0x1000_0004, &[1, 2, 3, 4], 0xe48b_ff56);
        let image = Uf2Image::parse(&input).unwrap();
        assert_eq!(
            image.materialize(0x1000_0000, 12, 0xff).unwrap(),
            [0xff, 0xff, 0xff, 0xff, 1, 2, 3, 4, 0xff, 0xff, 0xff, 0xff]
        );
        assert!(matches!(
            image.materialize(0x2000_0000, 12, 0xff),
            Err(Uf2Error::OutsideFlash { .. })
        ));
    }

    fn block(number: u32, count: u32, address: u32, data: &[u8], family: u32) -> Vec<u8> {
        let mut bytes = vec![0_u8; BLOCK_SIZE];
        put_u32(&mut bytes, 0, MAGIC_START_0);
        put_u32(&mut bytes, 4, MAGIC_START_1);
        put_u32(&mut bytes, 8, FLAG_FAMILY_ID);
        put_u32(&mut bytes, 12, address);
        put_u32(
            &mut bytes,
            16,
            u32::try_from(data.len()).expect("test payload fits"),
        );
        put_u32(&mut bytes, 20, number);
        put_u32(&mut bytes, 24, count);
        put_u32(&mut bytes, 28, family);
        bytes[DATA_OFFSET..DATA_OFFSET + data.len()].copy_from_slice(data);
        put_u32(&mut bytes, 508, MAGIC_END);
        bytes
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
