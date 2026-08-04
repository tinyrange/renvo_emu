//! ESP32-S3 external-memory manual AES-XTS encryption block.

use super::*;
use aes::{
    Aes128, Aes256,
    cipher::{Array, BlockCipherEncrypt, BlockSizeUser, KeyInit, consts::U16},
};

const STATE_IDLE: u32 = 0;
const STATE_DONE: u32 = 2;
const STATE_RELEASED: u32 = 3;
const DATE_RESET: u32 = 0x2020_0111;

#[derive(Clone)]
enum XtsKey {
    Aes128([u8; 32]),
    Aes256([u8; 64]),
}

struct Esp32S3XtsAesState {
    plaintext: [u32; 16],
    line_size: u32,
    destination: u32,
    physical_address: u32,
    state: u32,
    date: u32,
    key: XtsKey,
    ciphertext: Vec<u8>,
    operation_supported: bool,
}

impl Default for Esp32S3XtsAesState {
    fn default() -> Self {
        let mut key = [0_u8; 32];
        for (index, byte) in key.iter_mut().enumerate() {
            *byte = index as u8;
        }
        Self {
            plaintext: [0; 16],
            line_size: 0,
            destination: 0,
            physical_address: 0,
            state: STATE_IDLE,
            date: DATE_RESET,
            key: XtsKey::Aes128(key),
            ciphertext: Vec::new(),
            operation_supported: true,
        }
    }
}

impl Esp32S3XtsAesState {
    fn plaintext_bytes(&self) -> [u8; 64] {
        let mut bytes = [0_u8; 64];
        for (index, word) in self.plaintext.iter().copied().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    fn encrypt(&mut self) {
        let length = 16_usize << self.line_size;
        let register_bytes = self.plaintext_bytes();
        let start = self.physical_address as usize & 0x3f;
        let mut input = Vec::with_capacity(length);
        for index in 0..length {
            input.push(register_bytes[(start + index) & 0x3f]);
        }
        let tweak_value =
            (u128::from(self.destination) << 30) | u128::from(self.physical_address & 0x3fff_ff80);
        let block_index = ((self.physical_address & 0x7f) / 16) as usize;
        self.ciphertext = match &self.key {
            XtsKey::Aes128(key) => xts_encrypt_128(key, tweak_value, block_index, &input),
            XtsKey::Aes256(key) => xts_encrypt_256(key, tweak_value, block_index, &input),
        };
        self.operation_supported = true;
        self.state = STATE_DONE;
    }

    fn destroy(&mut self) {
        self.ciphertext.fill(0);
        self.ciphertext.clear();
        self.state = STATE_IDLE;
    }
}

fn advance_tweak(tweak: &mut [u8; 16]) {
    let mut carry = 0;
    for byte in tweak.iter_mut() {
        let next = *byte >> 7;
        *byte = (*byte << 1) | carry;
        carry = next;
    }
    if carry != 0 {
        tweak[0] ^= 0x87;
    }
}

fn encrypt_blocks<C>(cipher: &C, mut tweak: [u8; 16], input: &[u8]) -> Vec<u8>
where
    C: BlockCipherEncrypt + BlockSizeUser<BlockSize = U16>,
{
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(16) {
        let mut block_bytes = [0_u8; 16];
        for index in 0..16 {
            block_bytes[index] = chunk[index] ^ tweak[index];
        }
        let mut block = Array::from(block_bytes);
        cipher.encrypt_block(&mut block);
        for index in 0..16 {
            output.push(block[index] ^ tweak[index]);
        }
        advance_tweak(&mut tweak);
    }
    output
}

fn xts_encrypt_128(key: &[u8; 32], tweak_value: u128, block_index: usize, input: &[u8]) -> Vec<u8> {
    let data_key: [u8; 16] = key[..16].try_into().expect("AES-XTS data key fits");
    let tweak_key: [u8; 16] = key[16..].try_into().expect("AES-XTS tweak key fits");
    let data_cipher = Aes128::new(&Array::from(data_key));
    let tweak_cipher = Aes128::new(&Array::from(tweak_key));
    let mut tweak = Array::from(tweak_value.to_le_bytes());
    tweak_cipher.encrypt_block(&mut tweak);
    let mut tweak: [u8; 16] = tweak.into();
    for _ in 0..block_index {
        advance_tweak(&mut tweak);
    }
    encrypt_blocks(&data_cipher, tweak, input)
}

fn xts_encrypt_256(key: &[u8; 64], tweak_value: u128, block_index: usize, input: &[u8]) -> Vec<u8> {
    let data_key: [u8; 32] = key[..32].try_into().expect("AES-XTS data key fits");
    let tweak_key: [u8; 32] = key[32..].try_into().expect("AES-XTS tweak key fits");
    let data_cipher = Aes256::new(&Array::from(data_key));
    let tweak_cipher = Aes256::new(&Array::from(tweak_key));
    let mut tweak = Array::from(tweak_value.to_le_bytes());
    tweak_cipher.encrypt_block(&mut tweak);
    let mut tweak: [u8; 16] = tweak.into();
    for _ in 0..block_index {
        advance_tweak(&mut tweak);
    }
    encrypt_blocks(&data_cipher, tweak, input)
}

/// Host/SPI-facing view of released manual-encryption output.
#[derive(Clone)]
pub struct Esp32S3XtsAesHandle {
    state: Rc<RefCell<Esp32S3XtsAesState>>,
}

impl Esp32S3XtsAesHandle {
    /// Installs two AES-128 keys (data key followed by tweak key).
    pub fn set_key_128(&self, key: [u8; 32]) {
        self.state.borrow_mut().key = XtsKey::Aes128(key);
    }

    /// Installs two AES-256 keys (data key followed by tweak key).
    pub fn set_key_256(&self, key: [u8; 64]) {
        self.state.borrow_mut().key = XtsKey::Aes256(key);
    }

    /// Returns ciphertext only after software has issued RELEASE.
    pub fn released_ciphertext(&self) -> Option<Vec<u8>> {
        let state = self.state.borrow();
        (state.state == STATE_RELEASED).then(|| state.ciphertext.clone())
    }

    /// Reports whether the most recent trigger had valid configuration.
    pub fn operation_supported(&self) -> bool {
        self.state.borrow().operation_supported
    }
}

/// Functional ESP32-S3 XTS_AES manual-encryption peripheral.
pub struct Esp32S3XtsAes {
    name: String,
    state: Rc<RefCell<Esp32S3XtsAesState>>,
    system: EspSystemHandle,
}

impl Esp32S3XtsAes {
    /// Creates reset state coupled to the SYSTEM manual-encryption gate.
    pub fn new(name: impl Into<String>, system: EspSystemHandle) -> (Self, Esp32S3XtsAesHandle) {
        let state = Rc::new(RefCell::new(Esp32S3XtsAesState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                system,
            },
            Esp32S3XtsAesHandle { state },
        )
    }

    fn unsupported(&self, operation: &str, offset: u64) -> DeviceError {
        DeviceError::new(format!(
            "{} {operation} at reserved XTS_AES offset {offset:#x}",
            self.name
        ))
    }
}

impl Device for Esp32S3XtsAes {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 XTS_AES requires aligned word access",
            ));
        }
        let state = self.state.borrow();
        let value = match offset {
            0x00..=0x3c => state.plaintext[offset as usize / 4],
            0x40 => state.line_size,
            0x44 => state.destination,
            0x48 => state.physical_address,
            0x4c | 0x50 | 0x54 => 0,
            0x58 => state.state,
            0x5c => state.date,
            _ => return Err(self.unsupported("read", offset)),
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
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 XTS_AES requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 XTS_AES word write exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00..=0x3c if state.state == STATE_IDLE => {
                state.plaintext[offset as usize / 4] = value
            }
            0x40 if state.state == STATE_IDLE => state.line_size = value & 3,
            0x44 if state.state == STATE_IDLE => state.destination = value & 1,
            0x48 if state.state == STATE_IDLE => state.physical_address = value & 0x3fff_ffff,
            0x4c if value & 1 != 0 && state.state == STATE_IDLE => {
                if self.system.manual_encryption_enabled()
                    && state.destination == 0
                    && state.line_size <= 2
                {
                    state.encrypt();
                } else {
                    state.operation_supported = false;
                }
            }
            0x50 if value & 1 != 0 && state.state == STATE_DONE => state.state = STATE_RELEASED,
            0x54 if value & 1 != 0 => state.destroy(),
            0x58 => {}
            0x5c => state.date = value & 0x3fff_ffff,
            0x00..=0x5c => {}
            _ => return Err(self.unsupported("write", offset)),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let key = self.state.borrow().key.clone();
        *self.state.borrow_mut() = Esp32S3XtsAesState {
            key,
            ..Default::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(device: &mut impl Device, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn register_contract_state_machine_and_destroy_are_functional() {
        let (mut system, system_handle) = EspSystem::new("system");
        let (mut xts, handle) = Esp32S3XtsAes::new("xts", system_handle);
        assert_eq!(
            xts.read(0x5c, AccessWidth::Word, SimTime::ZERO),
            Ok(DATE_RESET.into())
        );
        assert!(xts.read(0x60, AccessWidth::Word, SimTime::ZERO).is_err());
        write(&mut xts, 0x4c, 1);
        assert!(!handle.operation_supported());
        write(&mut system, 0x4c, 1);
        for index in 0..4 {
            write(
                &mut xts,
                index * 4,
                0x0302_0100 + index as u32 * 0x0404_0404,
            );
        }
        write(&mut xts, 0x4c, 1);
        assert_eq!(
            xts.read(0x58, AccessWidth::Word, SimTime::ZERO),
            Ok(STATE_DONE.into())
        );
        assert!(handle.released_ciphertext().is_none());
        write(&mut xts, 0x50, 1);
        let ciphertext = handle
            .released_ciphertext()
            .expect("release exposes ciphertext");
        assert_eq!(ciphertext.len(), 16);
        // Independent OpenSSL 3.6 AES-128-XTS known-answer result for the
        // default 00..1f key, zero tweak, and 00..0f plaintext.
        assert_eq!(
            ciphertext,
            [
                0x74, 0xa1, 0x09, 0xaa, 0xbf, 0x19, 0x37, 0xc0, 0x22, 0xd1, 0x9d, 0xa4, 0xb9, 0x6c,
                0xbc, 0x40,
            ]
        );
        write(&mut xts, 0x54, 1);
        assert_eq!(
            xts.read(0x58, AccessWidth::Word, SimTime::ZERO),
            Ok(STATE_IDLE.into())
        );
        assert!(handle.released_ciphertext().is_none());
    }

    #[test]
    fn line_size_address_mapping_and_both_key_widths_are_deterministic() {
        let (mut system, system_handle) = EspSystem::new("system");
        write(&mut system, 0x4c, 1);
        let (mut xts, handle) = Esp32S3XtsAes::new("xts", system_handle);
        handle.set_key_256([0x5a; 64]);
        write(&mut xts, 0x40, 2);
        write(&mut xts, 0x48, 0x20);
        for index in 0..16 {
            write(&mut xts, index * 4, index as u32);
        }
        write(&mut xts, 0x4c, 1);
        write(&mut xts, 0x50, 1);
        assert_eq!(handle.released_ciphertext().unwrap().len(), 64);
    }
}
