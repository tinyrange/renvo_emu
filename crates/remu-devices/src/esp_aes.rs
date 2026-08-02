//! Functional ESP32-S3 AES accelerator.

use super::*;
use aes::{
    Aes128, Aes256,
    cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit},
};

/// Native ESP32-S3 AES register offsets from `hwcrypto_reg.h`.
pub const ESP32S3_AES_KEY_BASE: u64 = 0x00;
/// Native text input register window base.
pub const ESP32S3_AES_TEXT_IN_BASE: u64 = 0x20;
/// Native text output register window base.
pub const ESP32S3_AES_TEXT_OUT_BASE: u64 = 0x30;
/// AES mode register.
pub const ESP32S3_AES_MODE: u64 = 0x40;
/// AES trigger register.
pub const ESP32S3_AES_TRIGGER: u64 = 0x48;
/// AES busy state register.
pub const ESP32S3_AES_STATE: u64 = 0x4c;
/// Initialization-vector register window base.
pub const ESP32S3_AES_IV_MEM_BASE: u64 = 0x50;
/// GCM hash-subkey register window base.
pub const ESP32S3_AES_H_MEM_BASE: u64 = 0x60;
/// GCM J0 register window base.
pub const ESP32S3_AES_J0_MEM_BASE: u64 = 0x70;
/// GCM T0 register window base.
pub const ESP32S3_AES_T0_MEM_BASE: u64 = 0x80;
/// DMA working-mode register.
pub const ESP32S3_AES_DMA_ENABLE: u64 = 0x90;
/// DMA block-mode register.
pub const ESP32S3_AES_BLOCK_MODE: u64 = 0x94;
/// DMA block-count register.
pub const ESP32S3_AES_BLOCK_NUM: u64 = 0x98;
/// DMA counter-increment selection register.
pub const ESP32S3_AES_INC_SEL: u64 = 0x9c;
/// DMA additional-authenticated-data block count register.
pub const ESP32S3_AES_AAD_BLOCK_NUM: u64 = 0xa0;
/// DMA remainder-bit count register.
pub const ESP32S3_AES_REMAINDER_BIT_NUM: u64 = 0xa4;
/// AES continuation trigger register.
pub const ESP32S3_AES_CONTINUE: u64 = 0xa8;
/// AES interrupt clear register.
pub const ESP32S3_AES_INT_CLR: u64 = 0xac;
/// AES interrupt enable register.
pub const ESP32S3_AES_INT_ENA: u64 = 0xb0;
/// AES version register.
pub const ESP32S3_AES_DATE: u64 = 0xb4;
/// DMA exit configuration register.
pub const ESP32S3_AES_DMA_EXIT: u64 = 0xb8;

const AES_MODE_256: u32 = 0x02;
const AES_MODE_DECRYPT: u32 = 0x04;
const AES_SUPPORTED_MODES: [u32; 4] = [
    0,
    AES_MODE_256,
    AES_MODE_DECRYPT,
    AES_MODE_256 | AES_MODE_DECRYPT,
];

#[derive(Debug)]
struct Esp32S3AesState {
    key: [u32; 8],
    text_in: [u32; 4],
    text_out: [u32; 4],
    mode: u32,
    busy: bool,
    interrupt_enable: u32,
    interrupt_pending: bool,
    operation_supported: bool,
    registers: BTreeMap<u64, u32>,
}

impl Default for Esp32S3AesState {
    fn default() -> Self {
        Self {
            key: [0; 8],
            text_in: [0; 4],
            text_out: [0; 4],
            mode: 0,
            busy: false,
            interrupt_enable: 0,
            interrupt_pending: false,
            operation_supported: true,
            registers: BTreeMap::new(),
        }
    }
}

/// Host-side view of the ESP32-S3 AES accelerator.
#[derive(Clone)]
pub struct Esp32S3AesHandle {
    state: Arc<Mutex<Esp32S3AesState>>,
}

impl Esp32S3AesHandle {
    /// Returns the most recently completed 128-bit output block.
    pub fn text_out(&self) -> [u8; 16] {
        let state = self.state.lock().expect("ESP32-S3 AES lock poisoned");
        words_to_block(&state.text_out)
    }

    /// Returns true when a completion is pending and enabled.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP32-S3 AES lock poisoned");
        state.interrupt_pending && state.interrupt_enable != 0
    }

    /// Returns whether the most recent trigger used a supported mode.
    pub fn operation_supported(&self) -> bool {
        self.state
            .lock()
            .expect("ESP32-S3 AES lock poisoned")
            .operation_supported
    }
}

/// Functional ESP32-S3 AES peripheral.
pub struct Esp32S3Aes {
    name: String,
    state: Arc<Mutex<Esp32S3AesState>>,
    hub: SignalHub,
    text_out_signal: SignalId,
}

impl Esp32S3Aes {
    /// Creates the native register block and its host handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3AesHandle), SignalError> {
        let text_out_signal = hub.declare(
            "board.esp32s3.aes.text_out",
            signal_value_from_block(&[0; 16]),
            Some("last completed AES text output block".to_string()),
        )?;
        let state = Arc::new(Mutex::new(Esp32S3AesState::default()));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                text_out_signal,
            },
            Esp32S3AesHandle { state },
        ))
    }

    fn process(&mut self, at: SimTime) {
        let output = {
            let mut state = self.state.lock().expect("ESP32-S3 AES lock poisoned");
            state.busy = true;
            let mode = state.mode;
            let mut output = [0_u8; 16];
            let supported = AES_SUPPORTED_MODES.contains(&mode);
            if supported {
                let input = words_to_block(&state.text_in);
                let key_bytes = words_to_key(&state.key);
                if mode & AES_MODE_256 != 0 {
                    let key = Array::from(key_bytes);
                    let mut block = Array::from(input);
                    let cipher = Aes256::new(&key);
                    if mode & AES_MODE_DECRYPT != 0 {
                        cipher.decrypt_block(&mut block);
                    } else {
                        cipher.encrypt_block(&mut block);
                    }
                    output.copy_from_slice(&block);
                } else {
                    let key_bytes_128: [u8; 16] =
                        key_bytes[..16].try_into().expect("AES-128 key fits");
                    let key = Array::from(key_bytes_128);
                    let mut block = Array::from(input);
                    let cipher = Aes128::new(&key);
                    if mode & AES_MODE_DECRYPT != 0 {
                        cipher.decrypt_block(&mut block);
                    } else {
                        cipher.encrypt_block(&mut block);
                    }
                    output.copy_from_slice(&block);
                }
            }
            state.text_out = block_to_words(&output);
            state.operation_supported = supported;
            state.busy = false;
            state.interrupt_pending = true;
            output
        };
        let value = signal_value_from_block(&output);
        self.hub
            .set(self.text_out_signal, value, at)
            .expect("AES output signal remains declared");
    }

    fn read_word(state: &Esp32S3AesState, offset: u64) -> Result<u32, DeviceError> {
        if (ESP32S3_AES_KEY_BASE..ESP32S3_AES_KEY_BASE + 0x20).contains(&offset) {
            return Ok(
                state.key[usize::try_from((offset - ESP32S3_AES_KEY_BASE) / 4)
                    .expect("AES key index fits")],
            );
        }
        if (ESP32S3_AES_TEXT_IN_BASE..ESP32S3_AES_TEXT_IN_BASE + 0x10).contains(&offset) {
            return Ok(
                state.text_in[usize::try_from((offset - ESP32S3_AES_TEXT_IN_BASE) / 4)
                    .expect("AES input index fits")],
            );
        }
        if (ESP32S3_AES_TEXT_OUT_BASE..ESP32S3_AES_TEXT_OUT_BASE + 0x10).contains(&offset) {
            return Ok(
                state.text_out[usize::try_from((offset - ESP32S3_AES_TEXT_OUT_BASE) / 4)
                    .expect("AES output index fits")],
            );
        }
        Ok(match offset {
            ESP32S3_AES_MODE => state.mode,
            ESP32S3_AES_TRIGGER => 0,
            ESP32S3_AES_STATE => u32::from(state.busy),
            ESP32S3_AES_INT_CLR => 0,
            ESP32S3_AES_INT_ENA => state.interrupt_enable,
            ESP32S3_AES_DATE => 0,
            _ => state.registers.get(&offset).copied().unwrap_or_default(),
        })
    }
}

impl Device for Esp32S3Aes {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 AES requires aligned word access",
            ));
        }
        Ok(u64::from(Self::read_word(
            &self.state.lock().expect("ESP32-S3 AES lock poisoned"),
            offset,
        )?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 AES requires aligned word access",
            ));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("AES value fits in u32");
        let mut trigger = false;
        let mut continue_operation = false;
        {
            let mut state = self.state.lock().expect("ESP32-S3 AES lock poisoned");
            if (ESP32S3_AES_KEY_BASE..ESP32S3_AES_KEY_BASE + 0x20).contains(&offset) {
                state.key[usize::try_from((offset - ESP32S3_AES_KEY_BASE) / 4)
                    .expect("AES key index fits")] = value;
            } else if (ESP32S3_AES_TEXT_IN_BASE..ESP32S3_AES_TEXT_IN_BASE + 0x10).contains(&offset)
            {
                state.text_in[usize::try_from((offset - ESP32S3_AES_TEXT_IN_BASE) / 4)
                    .expect("AES input index fits")] = value;
            } else {
                match offset {
                    ESP32S3_AES_MODE => state.mode = value,
                    ESP32S3_AES_TRIGGER => trigger = value & 1 != 0,
                    ESP32S3_AES_CONTINUE => continue_operation = value & 1 != 0,
                    ESP32S3_AES_INT_CLR => {
                        if value != 0 {
                            state.interrupt_pending = false;
                        }
                    }
                    ESP32S3_AES_INT_ENA => state.interrupt_enable = value & 1,
                    ESP32S3_AES_STATE | ESP32S3_AES_DATE => {
                        return Err(DeviceError::new("ESP32-S3 AES register is read-only"));
                    }
                    _ => {
                        state.registers.insert(offset, value);
                    }
                }
            }
        }
        if trigger || continue_operation {
            self.process(at);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("ESP32-S3 AES lock poisoned") = Esp32S3AesState::default();
        self.hub
            .set(
                self.text_out_signal,
                signal_value_from_block(&[0; 16]),
                SimTime::ZERO,
            )
            .expect("AES output signal remains declared");
    }
}

fn words_to_key(words: &[u32; 8]) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (index, word) in words.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn words_to_block(words: &[u32; 4]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (index, word) in words.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn block_to_words(block: &[u8; 16]) -> [u32; 4] {
    std::array::from_fn(|index| {
        u32::from_le_bytes(
            block[index * 4..index * 4 + 4]
                .try_into()
                .expect("AES word fits"),
        )
    })
}

fn signal_value_from_block(block: &[u8; 16]) -> SignalValue {
    let bits = block
        .iter()
        .flat_map(|byte| {
            (0..8).map(move |bit| {
                if byte & (1 << bit) == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                }
            })
        })
        .collect();
    SignalValue::new(bits).expect("AES output signal is fixed-width")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY128: [u8; 16] = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    const KEY256: [u8; 32] = [
        0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe, 0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d, 0x77,
        0x81, 0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7, 0x2d, 0x98, 0x10, 0xa3, 0x09, 0x14,
        0xdf, 0xf4,
    ];
    const PLAINTEXT: [u8; 16] = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];

    fn write_block(device: &mut Esp32S3Aes, base: u64, block: &[u8]) {
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            device
                .write(
                    base + (index as u64 * 4),
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(chunk.try_into().expect("AES word fits"))),
                    SimTime::ZERO,
                )
                .unwrap();
        }
    }

    #[test]
    fn aes_128_encrypts_and_decrypts_native_text_window() {
        let hub = SignalHub::new();
        let (mut device, handle) = Esp32S3Aes::new("aes", hub).unwrap();
        write_block(&mut device, ESP32S3_AES_KEY_BASE, &KEY128);
        write_block(&mut device, ESP32S3_AES_TEXT_IN_BASE, &PLAINTEXT);
        device
            .write(ESP32S3_AES_INT_ENA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(ESP32S3_AES_TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            handle.text_out(),
            [
                0x3a, 0xd7, 0x7b, 0xb4, 0x0d, 0x7a, 0x36, 0x60, 0xa8, 0x9e, 0xca, 0xf3, 0x24, 0x66,
                0xef, 0x97,
            ]
        );
        assert!(handle.interrupt_pending());
        device
            .write(ESP32S3_AES_INT_CLR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());

        write_block(&mut device, ESP32S3_AES_TEXT_IN_BASE, &handle.text_out());
        device
            .write(
                ESP32S3_AES_MODE,
                AccessWidth::Word,
                AES_MODE_DECRYPT as u64,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(ESP32S3_AES_TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.text_out(), PLAINTEXT);
    }

    #[test]
    fn aes_256_and_unsupported_modes_are_explicit() {
        let hub = SignalHub::new();
        let (mut device, handle) = Esp32S3Aes::new("aes", hub).unwrap();
        write_block(&mut device, ESP32S3_AES_KEY_BASE, &KEY256);
        write_block(&mut device, ESP32S3_AES_TEXT_IN_BASE, &PLAINTEXT);
        device
            .write(
                ESP32S3_AES_MODE,
                AccessWidth::Word,
                AES_MODE_256 as u64,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(ESP32S3_AES_TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            handle.text_out(),
            [
                0xf3, 0xee, 0xd1, 0xbd, 0xb5, 0xd2, 0xa0, 0x3c, 0x06, 0x4b, 0x5a, 0x7e, 0x3d, 0xb1,
                0x81, 0xf8,
            ]
        );

        device
            .write(ESP32S3_AES_MODE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(ESP32S3_AES_TRIGGER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.text_out(), [0; 16]);
        assert!(!handle.operation_supported());
    }
}
