use super::*;
use sha2::{Digest, Sha224, Sha256};

const MESSAGE_START: u64 = 0x80;
const MESSAGE_END: u64 = 0xc0;
const HASH_START: u64 = 0x40;
const HASH_END: u64 = 0x80;

struct EspShaState {
    registers: Vec<u32>,
    message: [u8; 64],
    message_len: usize,
    hash: [u8; 64],
}

impl EspShaState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            message: [0; 64],
            message_len: 0,
            hash: [0; 64],
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.message.fill(0);
        self.hash.fill(0);
        self.message_len = 0;
        self.registers[0x2c / 4] = 538_972_713;
    }

    fn compute(&mut self) {
        self.hash.fill(0);
        match self.registers[0] & 0x7 {
            1 => {
                self.hash[..28].copy_from_slice(&Sha224::digest(&self.message[..self.message_len]))
            }
            2 => {
                self.hash[..32].copy_from_slice(&Sha256::digest(&self.message[..self.message_len]))
            }
            mode => {
                // The C6 baseline exposes SHA-224 and SHA-256. Keep unsupported
                // modes deterministic and visibly empty instead of pretending
                // to implement the SHA-1/384/512 accelerator variants.
                self.registers[0x18 / 4] = mode;
                return;
            }
        }
        self.registers[0x18 / 4] = 0;
        for (index, bytes) in self.hash.chunks_exact(4).enumerate() {
            self.registers[0x40 / 4 + index] = u32::from_le_bytes(bytes.try_into().unwrap());
        }
    }

    fn read(&self, offset: u64) -> u32 {
        match offset {
            HASH_START..HASH_END => {
                let index = (offset - HASH_START) as usize;
                u32::from_le_bytes(self.hash[index..index + 4].try_into().unwrap())
            }
            MESSAGE_START..MESSAGE_END => {
                let index = (offset - MESSAGE_START) as usize;
                u32::from_le_bytes(self.message[index..index + 4].try_into().unwrap())
            }
            _ => self.registers[(offset as usize) / 4],
        }
    }

    fn write(&mut self, offset: u64, value: u32) {
        match offset {
            0x10 => self.compute(),
            0x08 => {
                self.registers[0x08 / 4] = value & 0x3f;
                self.message_len = (value as usize).min(self.message.len());
            }
            0x24 => self.registers[0x18 / 4] = 0,
            MESSAGE_START..MESSAGE_END => {
                let index = (offset - MESSAGE_START) as usize;
                self.message[index..index + 4].copy_from_slice(&value.to_le_bytes());
                self.message_len = self.message_len.max(index + 4);
            }
            HASH_START..HASH_END => {}
            _ => self.registers[(offset as usize) / 4] = value,
        }
    }
}

/// Functional ESP32-C6 SHA accelerator.
pub struct EspSha {
    name: String,
    state: Rc<RefCell<EspShaState>>,
}

impl EspSha {
    /// Creates a reset SHA accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Rc::new(RefCell::new(EspShaState::new())),
        }
    }
}

impl Device for EspSha {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x1000 {
            return Err(DeviceError::new(
                "ESP32-C6 SHA requires aligned word access",
            ));
        }
        Ok(u64::from(self.state.borrow().read(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x1000 {
            return Err(DeviceError::new(
                "ESP32-C6 SHA requires aligned word access",
            ));
        }
        self.state.borrow_mut().write(offset, value as u32);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_register_path_matches_known_digest() {
        let mut sha = EspSha::new("sha");
        sha.write(
            0x80,
            AccessWidth::Word,
            u64::from(u32::from_le_bytes(*b"abc\0")),
            SimTime::ZERO,
        )
        .unwrap();
        sha.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        sha.write(0x08, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        sha.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let mut digest = Vec::new();
        for offset in (0x40..0x60).step_by(4) {
            let word = sha.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap() as u32;
            digest.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(
            digest[..32],
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }

    #[test]
    fn unsupported_mode_is_idle_and_deterministic() {
        let mut sha = EspSha::new("sha");
        sha.write(0x00, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        sha.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(sha.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap(), 7);
        assert_eq!(sha.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }
}
