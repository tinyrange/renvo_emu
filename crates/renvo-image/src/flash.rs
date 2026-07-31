use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

/// Immutable materialized flash contents with its content hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashSnapshot {
    /// Complete flash bytes.
    pub bytes: Vec<u8>,
    /// Lowercase SHA-256 of `bytes`.
    pub sha256: String,
}

/// Copy-on-write NOR flash storage with realistic erase/program constraints.
#[derive(Clone, Debug)]
pub struct PersistentFlash {
    base: Arc<[u8]>,
    overlay: BTreeMap<usize, u8>,
    erase_size: usize,
}

impl PersistentFlash {
    /// Creates erased flash of `size` bytes.
    pub fn erased(size: usize, erase_size: usize) -> Result<Self, FlashError> {
        Self::from_bytes(vec![0xff; size], erase_size)
    }

    /// Uses `bytes` as immutable base contents.
    pub fn from_bytes(bytes: Vec<u8>, erase_size: usize) -> Result<Self, FlashError> {
        if bytes.is_empty() {
            return Err(FlashError::Empty);
        }
        if erase_size == 0 || !erase_size.is_power_of_two() {
            return Err(FlashError::EraseSize(erase_size));
        }
        if bytes.len() % erase_size != 0 {
            return Err(FlashError::SizeAlignment {
                size: bytes.len(),
                erase_size,
            });
        }
        Ok(Self {
            base: bytes.into(),
            overlay: BTreeMap::new(),
            erase_size,
        })
    }

    /// Total flash capacity.
    pub fn len(&self) -> usize {
        self.base.len()
    }

    /// True only for an impossible zero-sized image.
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    /// Erase-block size.
    pub fn erase_size(&self) -> usize {
        self.erase_size
    }

    /// Number of bytes differing from the immutable base.
    pub fn changed_bytes(&self) -> usize {
        self.overlay.len()
    }

    /// Reads bytes from the current copy-on-write view.
    pub fn read(&self, offset: usize, output: &mut [u8]) -> Result<(), FlashError> {
        self.check_range(offset, output.len())?;
        for (index, byte) in output.iter_mut().enumerate() {
            let address = offset + index;
            *byte = self
                .overlay
                .get(&address)
                .copied()
                .unwrap_or(self.base[address]);
        }
        Ok(())
    }

    /// Programs bytes, allowing only one-to-zero bit transitions.
    pub fn program(&mut self, offset: usize, data: &[u8]) -> Result<(), FlashError> {
        self.check_range(offset, data.len())?;
        for (index, requested) in data.iter().copied().enumerate() {
            let address = offset + index;
            let current = self
                .overlay
                .get(&address)
                .copied()
                .unwrap_or(self.base[address]);
            if current & requested != requested {
                return Err(FlashError::NeedsErase {
                    offset: address,
                    current,
                    requested,
                });
            }
        }
        for (index, value) in data.iter().copied().enumerate() {
            self.set_overlay(offset + index, value);
        }
        Ok(())
    }

    /// Erases a block-aligned range to `0xff`.
    pub fn erase(&mut self, offset: usize, length: usize) -> Result<(), FlashError> {
        self.check_range(offset, length)?;
        if offset % self.erase_size != 0 || length % self.erase_size != 0 {
            return Err(FlashError::EraseAlignment {
                offset,
                length,
                erase_size: self.erase_size,
            });
        }
        for address in offset..offset + length {
            self.set_overlay(address, 0xff);
        }
        Ok(())
    }

    /// Discards all writes and erases, restoring the immutable base.
    pub fn discard_changes(&mut self) {
        self.overlay.clear();
    }

    /// Materializes current contents and computes their SHA-256.
    pub fn snapshot(&self) -> FlashSnapshot {
        let mut bytes = self.base.to_vec();
        for (&offset, &value) in &self.overlay {
            bytes[offset] = value;
        }
        let sha256 = hex::encode(Sha256::digest(&bytes));
        FlashSnapshot { bytes, sha256 }
    }

    fn set_overlay(&mut self, offset: usize, value: u8) {
        if self.base[offset] == value {
            self.overlay.remove(&offset);
        } else {
            self.overlay.insert(offset, value);
        }
    }

    fn check_range(&self, offset: usize, length: usize) -> Result<(), FlashError> {
        let end = offset.checked_add(length).ok_or(FlashError::OutOfRange {
            offset,
            length,
            size: self.len(),
        })?;
        if end > self.len() {
            return Err(FlashError::OutOfRange {
                offset,
                length,
                size: self.len(),
            });
        }
        Ok(())
    }
}

/// Persistent-flash operation failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FlashError {
    #[error("flash image must not be empty")]
    Empty,
    #[error("flash erase size {0} must be a non-zero power of two")]
    EraseSize(usize),
    #[error("flash size {size} is not aligned to erase size {erase_size}")]
    SizeAlignment { size: usize, erase_size: usize },
    #[error("flash range {offset:#x}+{length:#x} exceeds size {size:#x}")]
    OutOfRange {
        offset: usize,
        length: usize,
        size: usize,
    },
    #[error(
        "flash byte at {offset:#x} needs erase before {current:#04x} can become {requested:#04x}"
    )]
    NeedsErase {
        offset: usize,
        current: u8,
        requested: u8,
    },
    #[error("flash erase {offset:#x}+{length:#x} is not aligned to block size {erase_size:#x}")]
    EraseAlignment {
        offset: usize,
        length: usize,
        erase_size: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_nor_program_and_erase_rules() {
        let mut flash = PersistentFlash::erased(8192, 4096).unwrap();
        flash.program(10, &[0xf0, 0x0f]).unwrap();
        assert!(matches!(
            flash.program(10, &[0xff]),
            Err(FlashError::NeedsErase { .. })
        ));
        flash.erase(0, 4096).unwrap();
        let mut output = [0; 2];
        flash.read(10, &mut output).unwrap();
        assert_eq!(output, [0xff, 0xff]);
    }

    #[test]
    fn snapshot_and_discard_are_deterministic() {
        let mut flash = PersistentFlash::from_bytes(vec![0xff; 4096], 4096).unwrap();
        let initial = flash.snapshot();
        flash.program(0, &[0x12, 0x34]).unwrap();
        assert_ne!(flash.snapshot().sha256, initial.sha256);
        flash.discard_changes();
        assert_eq!(flash.snapshot(), initial);
    }
}
