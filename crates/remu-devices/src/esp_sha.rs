use super::*;
use sha2::block_api::compress256;

const MESSAGE_START: u64 = 0x80;
const MESSAGE_END: u64 = 0xc0;
const HASH_START: u64 = 0x40;
const HASH_END: u64 = 0x80;

struct EspShaState {
    registers: Vec<u32>,
    message: [u8; 64],
}

impl EspShaState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            message: [0; 64],
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.message.fill(0);
        self.registers[0x2c / 4] = 538_972_713;
    }

    fn initial_state(mode: u32) -> Option<[u32; 8]> {
        match mode {
            1 => Some([
                0xc105_9ed8,
                0x367c_d507,
                0x3070_dd17,
                0xf70e_5939,
                0xffc0_0b31,
                0x6858_1511,
                0x64f9_8fa7,
                0xbefa_4fa4,
            ]),
            2 => Some([
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ]),
            _ => None,
        }
    }

    fn compute(&mut self, first_block: bool) {
        let mode = self.registers[0] & 0x7;
        let Some(mut state) = Self::initial_state(mode).map(|initial| {
            if first_block {
                initial
            } else {
                let mut state = [0; 8];
                state.copy_from_slice(&self.registers[HASH_START as usize / 4..][..8]);
                state
            }
        }) else {
            // The C6 block accepts SHA1 and SHA2-512 family mode values in
            // the register definition, but this functional slice intentionally
            // implements only SHA-224 and SHA-256. Unsupported commands are
            // ignored and leave the engine idle.
            return;
        };

        self.registers[0x18 / 4] = 1;
        let block = self.message;
        compress256(&mut state, &[block]);
        self.registers[0x18 / 4] = 0;
        self.registers[HASH_START as usize / 4..][..8].copy_from_slice(&state);
    }

    fn read(&self, offset: u64) -> u32 {
        match offset {
            HASH_START..HASH_END => self.registers[offset as usize / 4],
            MESSAGE_START..MESSAGE_END => {
                let index = (offset - MESSAGE_START) as usize;
                u32::from_le_bytes(self.message[index..index + 4].try_into().unwrap())
            }
            0x00 => self.registers[0] & 0x7,
            0x04 => self.registers[0x04 / 4],
            0x08 => self.registers[0x08 / 4] & 0x3f,
            0x0c => self.registers[0x0c / 4] & 0x3f,
            0x18 => self.registers[0x18 / 4] & 1,
            0x28 => self.registers[0x28 / 4] & 1,
            0x2c => self.registers[0x2c / 4] & 0x3fff_ffff,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.registers[0] = value & 0x7,
            0x04 => self.registers[0x04 / 4] = value,
            0x08 => self.registers[0x08 / 4] = value & 0x3f,
            0x0c => self.registers[0x0c / 4] = value & 0x3f,
            0x10 => {
                if value & 1 != 0 {
                    self.compute(true);
                }
            }
            0x14 => {
                if value & 1 != 0 {
                    self.compute(false);
                }
            }
            0x24 => {
                // SHA_CLEAR_IRQ is a write-only command. There is no separate
                // interrupt-status register in this functional slice.
            }
            0x28 => self.registers[0x28 / 4] = value & 1,
            0x2c => self.registers[0x2c / 4] = value & 0x3fff_ffff,
            MESSAGE_START..MESSAGE_END => {
                let index = (offset - MESSAGE_START) as usize;
                self.message[index..index + 4].copy_from_slice(&value.to_le_bytes());
            }
            HASH_START..HASH_END => self.registers[offset as usize / 4] = value,
            // Start/continue DMA commands and reserved/read-only locations are
            // deliberately no-ops until a DMA/interrupt model is added.
            0x1c | 0x20 => {}
            _ => {}
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
    fn sha256_fixed_block_matches_known_digest() {
        let mut sha = EspSha::new("sha");
        let mut block = [0u8; 64];
        block[..3].copy_from_slice(b"abc");
        block[3] = 0x80;
        block[63] = 24;
        for (index, word) in block.chunks_exact(4).enumerate() {
            sha.write(
                0x80 + (index as u64 * 4),
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
        assert_eq!(
            (0..8)
                .map(|index| sha
                    .read(0x40 + index * 4, AccessWidth::Word, SimTime::ZERO)
                    .unwrap())
                .collect::<Vec<_>>(),
            [
                0xba78_16bf,
                0x8f01_cfea,
                0x4141_40de,
                0x5dae_2223,
                0xb003_61a3,
                0x9617_7a9c,
                0xb410_ff61,
                0xf200_15ad,
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
    fn register_masks_and_commands_match_native_layout() {
        let mut sha = EspSha::new("sha");
        sha.write(0x00, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x08, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x0c, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x28, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x2c, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x18, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        sha.write(0x10, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();

        assert_eq!(sha.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 7);
        assert_eq!(
            sha.read(0x08, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x3f
        );
        assert_eq!(
            sha.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x3f
        );
        assert_eq!(sha.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        assert_eq!(sha.read(0x28, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            sha.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x3fff_ffff
        );
    }

    #[test]
    fn continue_uses_hash_memory_as_intermediate_state() {
        let mut sha = EspSha::new("sha");
        sha.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        sha.write(0x40, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        sha.write(0x14, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_ne!(sha.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
    }
}
