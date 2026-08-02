use super::*;
use sha2::{Digest, Sha256};

const CSR_BSWAP: u32 = 1 << 12;
const CSR_DMA_SIZE_MASK: u32 = 0x300;
const CSR_ERROR: u32 = 1 << 4;
const CSR_SUM_VALID: u32 = 1 << 2;
const CSR_DATA_READY: u32 = 1 << 1;
const CSR_START: u32 = 1;

fn atomic_update(current: u32, alias: u64, value: u32) -> Result<u32, DeviceError> {
    match alias {
        0 => Ok(value),
        1 => Ok(current ^ value),
        2 => Ok(current | value),
        3 => Ok(current & !value),
        _ => Err(DeviceError::new("invalid RP2350 SHA-256 atomic alias")),
    }
}

fn aligned_word(width: AccessWidth, offset: u64) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || !width.is_aligned(offset) {
        Err(DeviceError::new(
            "RP2350 SHA-256 register requires aligned word access",
        ))
    } else {
        Ok(())
    }
}

const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// Functional RP2350 SHA-256 accelerator.
///
/// The model accepts byte, halfword, and word writes to WDATA, honours the BSWAP
/// control, and computes a digest as soon as a complete 512-bit block is
/// available. Firmware may provide either raw data or the standard padded
/// block sequence; both forms are useful for compiler and boot-ROM tests.
pub struct Rp2350Sha256 {
    name: String,
    bswap: bool,
    dma_size: u8,
    error: bool,
    sum_valid: bool,
    data_ready: bool,
    message: Vec<u8>,
    sum: [u32; 8],
}

impl Rp2350Sha256 {
    /// Creates a reset-state SHA-256 block.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bswap: true,
            dma_size: 2,
            error: false,
            sum_valid: true,
            data_ready: true,
            message: Vec::new(),
            sum: INITIAL_STATE,
        }
    }

    fn csr(&self) -> u32 {
        (u32::from(self.bswap) * CSR_BSWAP)
            | (u32::from(self.dma_size) << 8)
            | (u32::from(self.error) * CSR_ERROR)
            | (u32::from(self.sum_valid) * CSR_SUM_VALID)
            | (u32::from(self.data_ready) * CSR_DATA_READY)
    }

    fn start(&mut self) {
        self.error = false;
        self.sum_valid = true;
        self.data_ready = true;
        self.message.clear();
        self.sum = INITIAL_STATE;
    }

    fn logical_message(&self) -> &[u8] {
        if self.message.len() < 64 || self.message.len() % 64 != 0 {
            return &self.message;
        }
        let length_bytes = self.message.len() - 8;
        let bit_length = u64::from_be_bytes(
            self.message[length_bytes..]
                .try_into()
                .expect("SHA length trailer is eight bytes"),
        );
        let Ok(raw_length) = usize::try_from(bit_length / 8) else {
            return &self.message;
        };
        if bit_length % 8 != 0
            || raw_length >= length_bytes
            || self.message[raw_length] != 0x80
            || self.message[raw_length + 1..length_bytes]
                .iter()
                .any(|byte| *byte != 0)
        {
            return &self.message;
        }
        &self.message[..raw_length]
    }

    fn digest_complete_message(&mut self) {
        let digest = Sha256::digest(self.logical_message());
        for (index, word) in self.sum.iter_mut().enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes(
                digest[start..start + 4]
                    .try_into()
                    .expect("SHA digest word is four bytes"),
            );
        }
        self.sum_valid = true;
        self.data_ready = true;
    }

    fn write_data(&mut self, width: AccessWidth, value: u64) -> Result<(), DeviceError> {
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
        ) {
            return Err(DeviceError::new(
                "RP2350 SHA-256 WDATA supports byte, halfword, or word writes",
            ));
        }
        if !self.data_ready {
            self.error = true;
            return Ok(());
        }
        self.sum_valid = false;
        match width {
            AccessWidth::Byte => self.message.push(value as u8),
            AccessWidth::HalfWord => self
                .message
                .extend_from_slice(&(value as u16).to_le_bytes()),
            AccessWidth::Word => {
                let bytes = if self.bswap {
                    (value as u32).to_le_bytes()
                } else {
                    (value as u32).to_be_bytes()
                };
                self.message.extend_from_slice(&bytes);
            }
            AccessWidth::DoubleWord => unreachable!("checked SHA WDATA width"),
        }
        if self.message.len() % 64 == 0 {
            self.digest_complete_message();
        }
        Ok(())
    }
}

impl Device for Rp2350Sha256 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let register = offset & 0x0fff;
        let value = match register {
            0x00 => {
                aligned_word(width, offset)?;
                self.csr()
            }
            0x08..=0x24 if (register - 0x08) % 4 == 0 => {
                aligned_word(width, offset)?;
                self.sum[usize::try_from((register - 0x08) / 4).expect("SHA sum index fits")]
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 SHA-256 read at offset {register:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let register = offset & 0x0fff;
        let alias = (offset >> 12) & 3;
        let value = value & u64::from(u32::MAX);
        match register {
            0x00 => {
                aligned_word(width, offset)?;
                let current = (u32::from(self.bswap) * CSR_BSWAP) | (u32::from(self.dma_size) << 8);
                let updated = atomic_update(current, alias, value as u32)?;
                self.bswap = updated & CSR_BSWAP != 0;
                self.dma_size = ((updated & CSR_DMA_SIZE_MASK) >> 8) as u8;
                if alias == 0 && value as u32 & CSR_ERROR != 0 {
                    self.error = false;
                }
                if value as u32 & CSR_START != 0 {
                    self.start();
                }
            }
            0x04 => self.write_data(width, value)?,
            0x08..=0x24 if (register - 0x08) % 4 == 0 => {
                return Err(DeviceError::new(format!(
                    "RP2350 SHA-256 result at offset {register:#x} is read-only"
                )));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 SHA-256 write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}
