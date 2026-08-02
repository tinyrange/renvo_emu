//! Functional ESP32-C6 HMAC accelerator model.

use super::*;
use sha2::{Digest, Sha256};

const START: u64 = 0x40;
const PURPOSE: u64 = 0x44;
const KEY: u64 = 0x48;
const CONFIG_FINISH: u64 = 0x4c;
const MESSAGE_ONE: u64 = 0x50;
const MESSAGE_ING: u64 = 0x54;
const MESSAGE_END: u64 = 0x58;
const RESULT_FINISH: u64 = 0x5c;
const INVALIDATE_JTAG: u64 = 0x60;
const INVALIDATE_DS: u64 = 0x64;
const QUERY_ERROR: u64 = 0x68;
const QUERY_BUSY: u64 = 0x6c;
const MESSAGE_BASE: u64 = 0x80;
const MESSAGE_END_OFFSET: u64 = 0xc0;
const RESULT_BASE: u64 = 0xc0;
const RESULT_END: u64 = 0xe0;
const MESSAGE_PAD: u64 = 0xf0;
const ONE_BLOCK: u64 = 0xf4;
const SOFT_JTAG_CTRL: u64 = 0xf8;
const WR_JTAG: u64 = 0xfc;
const DATE: u64 = 0x1fc;

const PURPOSE_HMAC_UP: u32 = 8;
const MESSAGE_BYTES: usize = 64;
const RESULT_BYTES: usize = 32;

/// Deterministic functional subset of the ESP32-C6 HMAC peripheral.
///
/// The model implements the native key-purpose, message-memory, result-memory
/// and control/status registers for one padded SHA-256 block. Hardware eFuse
/// contents are not available to a standalone emulator, so key slots resolve
/// to deterministic synthetic keys; slot zero is 32 bytes of `0x42` and other
/// slots are derived from that value. Only the HMAC-up purpose is accepted.
/// Multi-block streaming, JTAG/DS output, and secure eFuse key semantics remain
/// explicit omissions rather than silently returning a false result.
pub struct EspHmac {
    name: String,
    registers: Vec<u32>,
    message: [u8; MESSAGE_BYTES],
    result: [u8; RESULT_BYTES],
    purpose: u32,
    key_slot: u32,
    configured: bool,
    error: bool,
    busy: bool,
}

impl EspHmac {
    /// Creates a reset HMAC accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        let mut device = Self {
            name: name.into(),
            registers: vec![0; 0x200 / 4],
            message: [0; MESSAGE_BYTES],
            result: [0; RESULT_BYTES],
            purpose: 0,
            key_slot: 0,
            configured: false,
            error: false,
            busy: false,
        };
        device.reset_state();
        device
    }

    fn reset_state(&mut self) {
        self.registers.fill(0);
        self.message.fill(0);
        self.result.fill(0);
        self.purpose = 0;
        self.key_slot = 0;
        self.configured = false;
        self.error = false;
        self.busy = false;
        // ESP-IDF's generated ESP32-C6 register header gives this reset date.
        self.registers[(DATE / 4) as usize] = 538_969_624;
    }

    fn set_word(&mut self, offset: u64, value: u32) {
        self.registers[(offset / 4) as usize] = value;
    }

    fn synthetic_key(slot: u32) -> [u8; 32] {
        let mut key = [0x42_u8; 32];
        let delta = (slot as u8).wrapping_mul(0x1d);
        for byte in &mut key {
            *byte = (*byte).wrapping_add(delta);
        }
        key
    }

    fn padded_message(&self) -> Result<&[u8], DeviceError> {
        let bit_length = u64::from_be_bytes(
            self.message[56..64]
                .try_into()
                .expect("HMAC length field is eight bytes"),
        );
        if bit_length % 8 != 0 || bit_length > 55 * 8 {
            return Err(DeviceError::new(
                "ESP32-C6 HMAC model requires one SHA-256 padded block",
            ));
        }
        let length = usize::try_from(bit_length / 8).expect("HMAC message length fits usize");
        if self.message[length] != 0x80
            || self.message[length + 1..56].iter().any(|byte| *byte != 0)
        {
            return Err(DeviceError::new(
                "ESP32-C6 HMAC message block has invalid SHA-256 padding",
            ));
        }
        Ok(&self.message[..length])
    }

    fn calculate(&mut self) -> Result<(), DeviceError> {
        if !self.configured || self.purpose != PURPOSE_HMAC_UP {
            self.error = true;
            return Ok(());
        }
        let message = self.padded_message()?.to_owned();
        let key = Self::synthetic_key(self.key_slot);
        let mut ipad = [0x36; 64];
        let mut opad = [0x5c; 64];
        for index in 0..key.len() {
            ipad[index] ^= key[index];
            opad[index] ^= key[index];
        }
        let mut inner = Sha256::new();
        inner.update(ipad);
        inner.update(&message);
        let inner_digest = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(opad);
        outer.update(inner_digest);
        self.result.copy_from_slice(&outer.finalize());
        self.error = false;
        Ok(())
    }

    fn read_word_bytes(bytes: &[u8], offset: u64) -> u32 {
        let index = usize::try_from(offset).expect("HMAC offset fits usize");
        u32::from_le_bytes(bytes[index..index + 4].try_into().expect("HMAC word fits"))
    }

    fn write_word_bytes(bytes: &mut [u8], offset: u64, value: u32) {
        let index = usize::try_from(offset).expect("HMAC offset fits usize");
        bytes[index..index + 4].copy_from_slice(&value.to_le_bytes());
    }
}

impl Device for EspHmac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 HMAC requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("HMAC offset fits");
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        let value = if (RESULT_BASE..RESULT_END).contains(&offset) {
            Self::read_word_bytes(&self.result, offset - RESULT_BASE)
        } else {
            match offset {
                QUERY_ERROR => u32::from(self.error),
                QUERY_BUSY => u32::from(self.busy),
                _ => self.registers[index],
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 HMAC requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("HMAC offset fits");
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            START => {
                self.set_word(offset, 0);
                if value & 1 != 0 {
                    self.message.fill(0);
                    self.result.fill(0);
                    self.configured = false;
                    self.error = false;
                    self.busy = false;
                }
            }
            PURPOSE => self.purpose = value & 0xf,
            KEY => self.key_slot = value & 0x7,
            CONFIG_FINISH => {
                self.configured =
                    value & 1 != 0 && self.purpose == PURPOSE_HMAC_UP && self.key_slot < 8;
                self.error = value & 1 != 0 && !self.configured;
            }
            MESSAGE_BASE..MESSAGE_END_OFFSET => {
                self.set_word(offset, value);
                Self::write_word_bytes(&mut self.message, offset - MESSAGE_BASE, value);
            }
            MESSAGE_ONE | MESSAGE_PAD | ONE_BLOCK => {
                self.set_word(offset, 0);
                if value & 1 != 0 {
                    self.busy = true;
                    let calculation = self.calculate();
                    self.busy = false;
                    calculation?;
                }
            }
            MESSAGE_ING | MESSAGE_END => {
                self.set_word(offset, 0);
                if value & 1 != 0 {
                    return Err(DeviceError::new(
                        "ESP32-C6 HMAC model does not implement multi-block streaming",
                    ));
                }
            }
            RESULT_FINISH | INVALIDATE_JTAG | INVALIDATE_DS => {
                self.set_word(offset, 0);
                if value & 1 != 0 || (offset == RESULT_FINISH && value == 2) {
                    self.result.fill(0);
                    self.busy = false;
                }
            }
            QUERY_ERROR | QUERY_BUSY | RESULT_BASE..RESULT_END => {}
            SOFT_JTAG_CTRL | WR_JTAG => self.set_word(offset, value),
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

    fn write_block(device: &mut EspHmac, message: &[u8]) {
        let mut block = [0_u8; MESSAGE_BYTES];
        block[..message.len()].copy_from_slice(message);
        block[message.len()] = 0x80;
        block[56..64].copy_from_slice(&((message.len() as u64) * 8).to_be_bytes());
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            device
                .write(
                    MESSAGE_BASE + (index as u64) * 4,
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(chunk.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
        }
    }

    fn read_result(device: &mut EspHmac) -> [u8; RESULT_BYTES] {
        let mut result = [0_u8; RESULT_BYTES];
        for index in 0..8 {
            let word = device
                .read(
                    RESULT_BASE + (index as u64) * 4,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap() as u32;
            result[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        result
    }

    #[test]
    fn one_block_hmac_matches_known_sha256_vector() {
        let mut device = EspHmac::new("hmac");
        device
            .write(START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(
                PURPOSE,
                AccessWidth::Word,
                PURPOSE_HMAC_UP.into(),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(KEY, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(CONFIG_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        write_block(&mut device, b"hello");
        device
            .write(MESSAGE_ONE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(QUERY_ERROR, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_eq!(
            device.read(QUERY_BUSY, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_eq!(
            read_result(&mut device),
            [
                0x80, 0xb8, 0xdb, 0x3c, 0xef, 0x47, 0x4c, 0x5a, 0xb5, 0xa6, 0x9b, 0x0a, 0x2f, 0xd2,
                0x82, 0x65, 0x59, 0xae, 0x25, 0xcd, 0x88, 0x83, 0xbd, 0x5e, 0xe8, 0x20, 0xf2, 0xe6,
                0xae, 0x50, 0xc5, 0x99,
            ]
        );
    }

    #[test]
    fn unsupported_purpose_is_reported_without_a_result() {
        let mut device = EspHmac::new("hmac");
        device
            .write(PURPOSE, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        device
            .write(KEY, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(CONFIG_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(QUERY_ERROR, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
    }
}
