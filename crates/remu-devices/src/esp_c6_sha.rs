use super::*;
use sha2::block_api::compress256;

const MESSAGE_START: u64 = 0x80;
const MESSAGE_END: u64 = 0xc0;
const HASH_START: u64 = 0x40;
const HASH_END: u64 = 0x80;

struct EspShaState {
    registers: Vec<u32>,
    message: [u8; 64],
    hash: [u32; 16],
}

impl EspShaState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            message: [0; 64],
            hash: [0; 16],
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.message.fill(0);
        self.hash.fill(0);
        self.registers[0x2c / 4] = 538_972_713;
    }

    fn process_block(&mut self, first: bool) {
        let initial = match self.registers[0] & 0x7 {
            1 => [
                0xc105_9ed8,
                0x367c_d507,
                0x3070_dd17,
                0xf70e_5939,
                0xffc0_0b31,
                0x6858_1511,
                0x64f9_8fa7,
                0xbefa_4fa4,
            ],
            2 => [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            _ => {
                self.hash.fill(0);
                return;
            }
        };
        let mut state = if first {
            initial
        } else {
            self.hash[..8]
                .try_into()
                .expect("SHA-256 state has 8 words")
        };
        compress256(&mut state, &[self.message]);
        self.hash[..8].copy_from_slice(&state);
        self.registers[0x18 / 4] = 0;
    }

    fn read(&self, offset: u64) -> u32 {
        match offset {
            HASH_START..HASH_END => {
                let index = (offset - HASH_START) as usize / 4;
                self.hash[index]
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
            0x10 => self.process_block(true),
            0x14 => self.process_block(false),
            0x08 => {
                self.registers[0x08 / 4] = value & 0x3f;
            }
            0x24 => self.registers[0x18 / 4] = 0,
            MESSAGE_START..MESSAGE_END => {
                let index = (offset - MESSAGE_START) as usize;
                self.message[index..index + 4].copy_from_slice(&value.to_le_bytes());
            }
            HASH_START..HASH_END => self.hash[(offset - HASH_START) as usize / 4] = value,
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
        let mut block = [0_u8; 64];
        block[..3].copy_from_slice(b"abc");
        block[3] = 0x80;
        block[56..].copy_from_slice(&24_u64.to_be_bytes());
        for (index, word) in block.chunks_exact(4).enumerate() {
            sha.write(
                0x80 + (index * 4) as u64,
                AccessWidth::Word,
                u64::from(u32::from_le_bytes(word.try_into().unwrap())),
                SimTime::ZERO,
            )
            .unwrap();
        }
        sha.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        sha.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let mut digest = Vec::new();
        for offset in (0x40..0x60).step_by(4) {
            let word = sha.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap() as u32;
            digest.extend_from_slice(&word.to_be_bytes());
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
        assert_eq!(sha.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        assert_eq!(sha.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }

    #[test]
    fn sha256_continue_uses_the_hardware_digest_state() {
        use sha2::{Digest, Sha256};

        let message = [0x5a_u8; 80];
        let mut blocks = [[0_u8; 64]; 2];
        blocks[0].copy_from_slice(&message[..64]);
        blocks[1][..16].copy_from_slice(&message[64..]);
        blocks[1][16] = 0x80;
        blocks[1][56..].copy_from_slice(&(message.len() as u64 * 8).to_be_bytes());

        let mut sha = EspSha::new("sha");
        sha.write(0, AccessWidth::Word, 2, SimTime::ZERO).unwrap();
        for (block_index, block) in blocks.iter().enumerate() {
            for (index, word) in block.chunks_exact(4).enumerate() {
                sha.write(
                    0x80 + (index * 4) as u64,
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(word.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
            }
            sha.write(
                if block_index == 0 { 0x10 } else { 0x14 },
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        }
        let mut observed = Vec::new();
        for offset in (0x40..0x60).step_by(4) {
            let word = sha.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap() as u32;
            observed.extend_from_slice(&word.to_be_bytes());
        }
        assert_eq!(observed, Sha256::digest(message).as_slice());
    }
}
