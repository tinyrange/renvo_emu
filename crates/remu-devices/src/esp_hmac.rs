use super::*;
use sha2::{Digest, Sha256};

const HMAC_SET_START: u64 = 0x40;
const HMAC_SET_PURPOSE: u64 = 0x44;
const HMAC_SET_KEY: u64 = 0x48;
const HMAC_SET_FINISH: u64 = 0x4c;
const HMAC_MESSAGE_ONE: u64 = 0x50;
const HMAC_MESSAGE_ING: u64 = 0x54;
const HMAC_MESSAGE_END: u64 = 0x58;
const HMAC_RESULT_FINISH: u64 = 0x5c;
const HMAC_INVALIDATE_JTAG: u64 = 0x60;
const HMAC_INVALIDATE_DS: u64 = 0x64;
const HMAC_QUERY_ERROR: u64 = 0x68;
const HMAC_QUERY_BUSY: u64 = 0x6c;
const HMAC_WDATA_BASE: u64 = 0x80;
const HMAC_RDATA_BASE: u64 = 0xc0;
const HMAC_MESSAGE_PAD: u64 = 0xf0;
const HMAC_ONE_BLOCK: u64 = 0xf4;

const HMAC_KEY_PURPOSE_DOWN_ALL: u32 = 5;
const HMAC_KEY_PURPOSE_UP: u32 = 8;
const HMAC_BLOCK_BYTES: usize = 64;
const HMAC_DIGEST_BYTES: usize = 32;

struct EspHmacState {
    registers: Vec<u32>,
    message: Vec<u8>,
    result: [u8; HMAC_DIGEST_BYTES],
    key_id: u32,
    purpose: u32,
    started: bool,
    config_finished: bool,
    error: u32,
}

impl EspHmacState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            message: Vec::new(),
            result: [0; HMAC_DIGEST_BYTES],
            key_id: 0,
            purpose: 0,
            started: false,
            config_finished: false,
            error: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.message.clear();
        self.result = [0; HMAC_DIGEST_BYTES];
        self.key_id = 0;
        self.purpose = 0;
        self.started = false;
        self.config_finished = false;
        self.error = 0;
    }

    fn block(&self) -> [u8; HMAC_BLOCK_BYTES] {
        let mut block = [0_u8; HMAC_BLOCK_BYTES];
        for (index, word) in self.registers[(HMAC_WDATA_BASE / 4) as usize
            ..((HMAC_WDATA_BASE + HMAC_BLOCK_BYTES as u64) / 4) as usize]
            .iter()
            .enumerate()
        {
            block[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        block
    }

    fn synthetic_key(key_id: u32) -> [u8; HMAC_DIGEST_BYTES] {
        let mut seed = b"renvo-esp32s3-hmac-efuse-key-v1".to_vec();
        seed.extend_from_slice(&key_id.to_le_bytes());
        Sha256::digest(seed).into()
    }

    fn digest(&self, message: &[u8]) -> [u8; HMAC_DIGEST_BYTES] {
        let mut key = [0_u8; HMAC_BLOCK_BYTES];
        key[..HMAC_DIGEST_BYTES].copy_from_slice(&Self::synthetic_key(self.key_id));
        let mut inner_pad = [0_u8; HMAC_BLOCK_BYTES];
        let mut outer_pad = [0_u8; HMAC_BLOCK_BYTES];
        for index in 0..HMAC_BLOCK_BYTES {
            inner_pad[index] = key[index] ^ 0x36;
            outer_pad[index] = key[index] ^ 0x5c;
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        inner.update(message);
        let inner_digest = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        outer.update(inner_digest);
        outer.finalize().into()
    }

    fn publish_result(&mut self) {
        self.result = self.digest(&self.message);
        for (index, chunk) in self.result.chunks_exact(4).enumerate() {
            self.registers[(HMAC_RDATA_BASE / 4) as usize + index] =
                u32::from_le_bytes(chunk.try_into().expect("digest word is four bytes"));
        }
    }

    fn command_error(&mut self, error: u32) {
        if self.error == 0 {
            self.error = error;
        }
    }

    fn append_block(&mut self) {
        self.message.extend_from_slice(&self.block());
    }

    fn finish_configuration(&mut self) {
        self.config_finished = self.started
            && (HMAC_KEY_PURPOSE_DOWN_ALL..=HMAC_KEY_PURPOSE_UP).contains(&self.purpose);
        if !self.started {
            self.command_error(2);
        } else if !self.config_finished {
            self.command_error(1);
        }
    }

    fn execute_message_command(&mut self, offset: u64) {
        if !self.config_finished {
            self.command_error(3);
            return;
        }
        match offset {
            HMAC_MESSAGE_ING => self.append_block(),
            HMAC_MESSAGE_ONE | HMAC_MESSAGE_END | HMAC_MESSAGE_PAD | HMAC_ONE_BLOCK => {
                self.append_block();
                self.publish_result();
            }
            _ => {}
        }
    }
}

/// Functional ESP32-S3 HMAC-SHA256 accelerator.
///
/// The native command and data-window layout is retained. Since an emulator
/// has no physical eFuse key, each selected key slot derives a stable synthetic
/// 256-bit key; this keeps compiler and firmware tests deterministic while
/// explicitly avoiding a claim of secure-key or eFuse fidelity.
pub struct EspHmac {
    name: String,
    state: EspHmacState,
}

impl EspHmac {
    /// Creates an idle HMAC register block.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspHmacState::new(),
        }
    }
}

impl Device for EspHmac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP HMAC requires aligned word access"));
        }
        match offset {
            HMAC_QUERY_ERROR => Ok(u64::from(self.state.error)),
            HMAC_QUERY_BUSY => Ok(0),
            _ => self
                .state
                .registers
                .get(usize::try_from(offset / 4).expect("HMAC offset fits"))
                .copied()
                .map(u64::from)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name))),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP HMAC requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("HMAC offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        self.state.registers[index] = value;
        match offset {
            HMAC_SET_START => {
                self.state.started = value != 0;
                self.state.config_finished = false;
                self.state.message.clear();
                self.state.result = [0; HMAC_DIGEST_BYTES];
                self.state.error = 0;
            }
            HMAC_SET_PURPOSE => self.state.purpose = value,
            HMAC_SET_KEY => self.state.key_id = value,
            HMAC_SET_FINISH => self.state.finish_configuration(),
            HMAC_MESSAGE_ING | HMAC_MESSAGE_ONE | HMAC_MESSAGE_END | HMAC_MESSAGE_PAD
            | HMAC_ONE_BLOCK => self.state.execute_message_command(offset),
            HMAC_RESULT_FINISH => {
                self.state.message.clear();
                self.state.result = [0; HMAC_DIGEST_BYTES];
                for register in &mut self.state.registers
                    [(HMAC_RDATA_BASE / 4) as usize..((HMAC_RDATA_BASE + 0x20) / 4) as usize]
                {
                    *register = 0;
                }
            }
            HMAC_INVALIDATE_JTAG | HMAC_INVALIDATE_DS => self.state.result = [0; HMAC_DIGEST_BYTES],
            HMAC_QUERY_ERROR | HMAC_QUERY_BUSY | HMAC_RDATA_BASE..=0xdf => {}
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_native_one_block_hmac_with_a_deterministic_key_slot() {
        let mut device = EspHmac::new("hmac");
        device
            .write(HMAC_SET_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_SET_PURPOSE, AccessWidth::Word, 8, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_SET_KEY, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_SET_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let message = (0_u8..64).collect::<Vec<_>>();
        for (index, chunk) in message.chunks_exact(4).enumerate() {
            device
                .write(
                    HMAC_WDATA_BASE + (index as u64 * 4),
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(chunk.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
        }
        device
            .write(HMAC_MESSAGE_ONE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();

        let expected = device.state.digest(&message);
        let actual = (0..8)
            .flat_map(|index| {
                u32::try_from(
                    device
                        .read(
                            HMAC_RDATA_BASE + index * 4,
                            AccessWidth::Word,
                            SimTime::ZERO,
                        )
                        .unwrap(),
                )
                .unwrap()
                .to_le_bytes()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(
            device.read(HMAC_QUERY_ERROR, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_eq!(
            device.read(HMAC_QUERY_BUSY, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn rejects_hmac_commands_before_a_valid_configuration() {
        let mut device = EspHmac::new("hmac");
        device
            .write(HMAC_SET_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_SET_PURPOSE, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_SET_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(HMAC_MESSAGE_ONE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_ne!(
            device.read(HMAC_QUERY_ERROR, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }
}
