//! Functional ESP32-C6 AES accelerator model.

use super::*;
use aes::cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes::{Aes128, Aes256};

const KEY_BASE: u64 = 0x00;
const KEY_END: u64 = 0x20;
const TEXT_IN_BASE: u64 = 0x20;
const TEXT_IN_END: u64 = 0x30;
const TEXT_OUT_BASE: u64 = 0x30;
const TEXT_OUT_END: u64 = 0x40;
const MODE: u64 = 0x40;
const ENDIAN: u64 = 0x44;
const TRIGGER: u64 = 0x48;
const STATE: u64 = 0x4c;
const IV_BASE: u64 = 0x50;
const T0_END: u64 = 0x90;
const DMA_ENABLE: u64 = 0x90;
const BLOCK_MODE: u64 = 0x94;
const BLOCK_NUM: u64 = 0x98;
const INC_SEL: u64 = 0x9c;
const AAD_BLOCK_NUM: u64 = 0xa0;
const REMAINDER_BIT_NUM: u64 = 0xa4;
const CONTINUE: u64 = 0xa8;
const INT_CLEAR: u64 = 0xac;
const INT_ENABLE: u64 = 0xb0;
const DATE: u64 = 0xb4;
const DMA_EXIT: u64 = 0xb8;

const MODE_ENCRYPT_128: u32 = 0;
const MODE_ENCRYPT_256: u32 = 2;
const MODE_DECRYPT_128: u32 = 4;
const MODE_DECRYPT_256: u32 = 6;
const STATE_IDLE: u32 = 0;
const STATE_DONE: u32 = 2;

/// Deterministic functional subset of the ESP32-C6 AES peripheral.
///
/// The native single-block key, input, output, mode, trigger and state
/// registers are implemented for AES-128 and AES-256 ECB transforms. The
/// register words use the same little-endian memory layout as Espressif's
/// `aes_ll_write_key` and `aes_ll_write_block` helpers. DMA and chained block
/// modes are retained as configuration registers but are rejected when a
/// transform is triggered; this keeps unsupported behavior explicit.
pub struct EspAes {
    name: String,
    registers: Vec<u32>,
    key: [u8; 32],
    input: [u8; 16],
    output: [u8; 16],
    state: u32,
}

impl EspAes {
    /// Creates a reset AES accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        let mut device = Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            key: [0; 32],
            input: [0; 16],
            output: [0; 16],
            state: STATE_IDLE,
        };
        device.reset_state();
        device
    }

    fn reset_state(&mut self) {
        self.registers.fill(0);
        self.key.fill(0);
        self.input.fill(0);
        self.output.fill(0);
        self.state = STATE_IDLE;
        // ESP-IDF's generated ESP32-C6 register header gives this reset date.
        self.registers[(DATE / 4) as usize] = 538_513_936;
    }

    fn register(&self, offset: u64) -> u32 {
        self.registers[(offset / 4) as usize]
    }

    fn set_word(&mut self, offset: u64, value: u32) {
        self.registers[(offset / 4) as usize] = value;
    }

    fn transform(&mut self) -> Result<(), DeviceError> {
        if self.register(DMA_ENABLE) & 1 != 0 || self.register(BLOCK_MODE) & 0x7 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 AES model supports only non-DMA ECB transforms",
            ));
        }
        let mode = self.register(MODE) & 0x7;
        let mut block = Array::from(self.input);
        match mode {
            MODE_ENCRYPT_128 => {
                let cipher = Aes128::new_from_slice(&self.key[..16])
                    .map_err(|_| DeviceError::new("invalid AES-128 key"))?;
                cipher.encrypt_block(&mut block);
            }
            MODE_ENCRYPT_256 => {
                let cipher = Aes256::new_from_slice(&self.key)
                    .map_err(|_| DeviceError::new("invalid AES-256 key"))?;
                cipher.encrypt_block(&mut block);
            }
            MODE_DECRYPT_128 => {
                let cipher = Aes128::new_from_slice(&self.key[..16])
                    .map_err(|_| DeviceError::new("invalid AES-128 key"))?;
                cipher.decrypt_block(&mut block);
            }
            MODE_DECRYPT_256 => {
                let cipher = Aes256::new_from_slice(&self.key)
                    .map_err(|_| DeviceError::new("invalid AES-256 key"))?;
                cipher.decrypt_block(&mut block);
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unsupported ESP32-C6 AES mode {mode}"
                )));
            }
        }
        self.output.copy_from_slice(&block);
        self.state = STATE_DONE;
        Ok(())
    }

    fn read_word_bytes(bytes: &[u8], offset: u64) -> u32 {
        let index = usize::try_from(offset).expect("AES offset fits usize");
        u32::from_le_bytes(bytes[index..index + 4].try_into().expect("AES word fits"))
    }

    fn write_word_bytes(bytes: &mut [u8], offset: u64, value: u32) {
        let index = usize::try_from(offset).expect("AES offset fits usize");
        bytes[index..index + 4].copy_from_slice(&value.to_le_bytes());
    }
}

impl Device for EspAes {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 AES requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("AES offset fits");
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        let value = if (TEXT_OUT_BASE..TEXT_OUT_END).contains(&offset) {
            Self::read_word_bytes(&self.output, offset - TEXT_OUT_BASE)
        } else if (STATE..=STATE).contains(&offset) {
            self.state
        } else {
            self.registers[index]
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 AES requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("AES offset fits");
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            KEY_BASE..KEY_END => {
                self.set_word(offset, value);
                Self::write_word_bytes(&mut self.key, offset - KEY_BASE, value);
            }
            TEXT_IN_BASE..TEXT_IN_END => {
                self.set_word(offset, value);
                Self::write_word_bytes(&mut self.input, offset - TEXT_IN_BASE, value);
            }
            TEXT_OUT_BASE..TEXT_OUT_END | STATE => {}
            MODE => self.set_word(offset, value & 0x7),
            ENDIAN => self.set_word(offset, value & 0x3f),
            TRIGGER => {
                self.set_word(offset, 0);
                if value & 1 != 0 {
                    self.transform()?;
                }
            }
            INT_CLEAR => {
                self.set_word(offset, 0);
                if value & 1 != 0 {
                    self.state = STATE_IDLE;
                }
            }
            DMA_ENABLE => self.set_word(offset, value & 1),
            BLOCK_MODE => self.set_word(offset, value & 0x7),
            REMAINDER_BIT_NUM => self.set_word(offset, value & 0x7f),
            INT_ENABLE => self.set_word(offset, value & 1),
            IV_BASE..T0_END | BLOCK_NUM | INC_SEL | AAD_BLOCK_NUM | CONTINUE | DMA_EXIT => {
                self.set_word(offset, value);
            }
            _ => self.set_word(offset, value),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bytes(device: &mut EspAes, base: u64, bytes: &[u8]) {
        for (index, chunk) in bytes.chunks_exact(4).enumerate() {
            device
                .write(
                    base + (index as u64) * 4,
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(chunk.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
        }
    }

    fn read_bytes(device: &mut EspAes) -> [u8; 16] {
        let mut bytes = [0; 16];
        for index in 0..4 {
            let value = device
                .read(
                    TEXT_OUT_BASE + (index as u64) * 4,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap() as u32;
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn aes128_ecb_matches_nist_vector_and_reports_done() {
        let mut device = EspAes::new("aes");
        write_bytes(
            &mut device,
            KEY_BASE,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        );
        write_bytes(
            &mut device,
            TEXT_IN_BASE,
            &[
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ],
        );
        device
            .write(TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(device.read(STATE, AccessWidth::Word, SimTime::ZERO), Ok(2));
        assert_eq!(
            read_bytes(&mut device),
            [
                0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
                0xc5, 0x5a
            ]
        );
        device
            .write(INT_CLEAR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(device.read(STATE, AccessWidth::Word, SimTime::ZERO), Ok(0));
    }

    #[test]
    fn aes256_decrypt_matches_nist_vector() {
        let mut device = EspAes::new("aes");
        write_bytes(
            &mut device,
            KEY_BASE,
            &[
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        );
        write_bytes(
            &mut device,
            TEXT_IN_BASE,
            &[
                0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49,
                0x60, 0x89,
            ],
        );
        device
            .write(
                MODE,
                AccessWidth::Word,
                u64::from(MODE_DECRYPT_256),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            read_bytes(&mut device),
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
    }

    #[test]
    fn chained_modes_are_rejected_explicitly() {
        let mut device = EspAes::new("aes");
        device
            .write(BLOCK_MODE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(
            device
                .write(TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
    }
}
