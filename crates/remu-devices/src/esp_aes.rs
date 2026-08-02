//! Functional ESP32-S3 AES accelerator.

use super::*;
use aes::{
    Aes128, Aes256,
    cipher::{Array, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit},
};

/// Native ESP32-S3 AES register identifiers from `hwcrypto_reg.h`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Esp32S3AesRegister {
    Key0 = 0x00,
    Key1 = 0x04,
    Key2 = 0x08,
    Key3 = 0x0c,
    Key4 = 0x10,
    Key5 = 0x14,
    Key6 = 0x18,
    Key7 = 0x1c,
    TextIn0 = 0x20,
    TextIn1 = 0x24,
    TextIn2 = 0x28,
    TextIn3 = 0x2c,
    TextOut0 = 0x30,
    TextOut1 = 0x34,
    TextOut2 = 0x38,
    TextOut3 = 0x3c,
    Mode = 0x40,
    Endian = 0x44,
    Trigger = 0x48,
    State = 0x4c,
    Iv0 = 0x50,
    Iv1 = 0x54,
    Iv2 = 0x58,
    Iv3 = 0x5c,
    H0 = 0x60,
    H1 = 0x64,
    H2 = 0x68,
    H3 = 0x6c,
    J0_0 = 0x70,
    J0_1 = 0x74,
    J0_2 = 0x78,
    J0_3 = 0x7c,
    T0 = 0x80,
    T1 = 0x84,
    T2 = 0x88,
    T3 = 0x8c,
    DmaEnable = 0x90,
    BlockMode = 0x94,
    BlockNum = 0x98,
    IncSel = 0x9c,
    AadBlockNum = 0xa0,
    BitValidNum = 0xa4,
    Continue = 0xa8,
    IntClear = 0xac,
    IntEna = 0xb0,
    Date = 0xb4,
    DmaExit = 0xb8,
}

impl Esp32S3AesRegister {
    /// Returns the byte offset in the AES peripheral page.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Resolves a native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Key0,
            0x04 => Self::Key1,
            0x08 => Self::Key2,
            0x0c => Self::Key3,
            0x10 => Self::Key4,
            0x14 => Self::Key5,
            0x18 => Self::Key6,
            0x1c => Self::Key7,
            0x20 => Self::TextIn0,
            0x24 => Self::TextIn1,
            0x28 => Self::TextIn2,
            0x2c => Self::TextIn3,
            0x30 => Self::TextOut0,
            0x34 => Self::TextOut1,
            0x38 => Self::TextOut2,
            0x3c => Self::TextOut3,
            0x40 => Self::Mode,
            0x44 => Self::Endian,
            0x48 => Self::Trigger,
            0x4c => Self::State,
            0x50 => Self::Iv0,
            0x54 => Self::Iv1,
            0x58 => Self::Iv2,
            0x5c => Self::Iv3,
            0x60 => Self::H0,
            0x64 => Self::H1,
            0x68 => Self::H2,
            0x6c => Self::H3,
            0x70 => Self::J0_0,
            0x74 => Self::J0_1,
            0x78 => Self::J0_2,
            0x7c => Self::J0_3,
            0x80 => Self::T0,
            0x84 => Self::T1,
            0x88 => Self::T2,
            0x8c => Self::T3,
            0x90 => Self::DmaEnable,
            0x94 => Self::BlockMode,
            0x98 => Self::BlockNum,
            0x9c => Self::IncSel,
            0xa0 => Self::AadBlockNum,
            0xa4 => Self::BitValidNum,
            0xa8 => Self::Continue,
            0xac => Self::IntClear,
            0xb0 => Self::IntEna,
            0xb4 => Self::Date,
            0xb8 => Self::DmaExit,
            _ => return None,
        })
    }

    fn key_index(self) -> Option<usize> {
        Some(match self {
            Self::Key0 => 0,
            Self::Key1 => 1,
            Self::Key2 => 2,
            Self::Key3 => 3,
            Self::Key4 => 4,
            Self::Key5 => 5,
            Self::Key6 => 6,
            Self::Key7 => 7,
            _ => return None,
        })
    }

    fn text_in_index(self) -> Option<usize> {
        Some(match self {
            Self::TextIn0 => 0,
            Self::TextIn1 => 1,
            Self::TextIn2 => 2,
            Self::TextIn3 => 3,
            _ => return None,
        })
    }

    fn text_out_index(self) -> Option<usize> {
        Some(match self {
            Self::TextOut0 => 0,
            Self::TextOut1 => 1,
            Self::TextOut2 => 2,
            Self::TextOut3 => 3,
            _ => return None,
        })
    }

    const fn read_mask(self) -> u32 {
        match self {
            Self::Mode => 0x7,
            Self::Endian | Self::State | Self::DmaEnable | Self::IncSel | Self::IntEna => 1,
            Self::BlockMode => 0x7,
            Self::Trigger | Self::Continue | Self::IntClear | Self::DmaExit => 0,
            _ => u32::MAX,
        }
    }

    const fn write_mask(self) -> u32 {
        match self {
            Self::TextOut0
            | Self::TextOut1
            | Self::TextOut2
            | Self::TextOut3
            | Self::State
            | Self::Date => 0,
            Self::Mode => 0x7,
            Self::Endian | Self::DmaEnable | Self::IncSel | Self::IntEna => 1,
            Self::BlockMode => 0x7,
            Self::Trigger | Self::Continue | Self::IntClear | Self::DmaExit => 1,
            _ => u32::MAX,
        }
    }
}

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
    registers: BTreeMap<Esp32S3AesRegister, u32>,
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

    fn read_word(state: &Esp32S3AesState, register: Esp32S3AesRegister) -> u32 {
        if let Some(index) = register.key_index() {
            return state.key[index];
        }
        if let Some(index) = register.text_in_index() {
            return state.text_in[index];
        }
        if let Some(index) = register.text_out_index() {
            return state.text_out[index];
        }
        match register {
            Esp32S3AesRegister::Mode => state.mode,
            Esp32S3AesRegister::Trigger
            | Esp32S3AesRegister::Continue
            | Esp32S3AesRegister::IntClear => 0,
            Esp32S3AesRegister::State => u32::from(state.busy),
            Esp32S3AesRegister::IntEna => state.interrupt_enable,
            Esp32S3AesRegister::Date => 0,
            _ => state.registers.get(&register).copied().unwrap_or_default() & register.read_mask(),
        }
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
        let register = Esp32S3AesRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 AES register offset {offset:#x}"
            ))
        })?;
        Ok(u64::from(Self::read_word(
            &self.state.lock().expect("ESP32-S3 AES lock poisoned"),
            register,
        )))
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
        let register = Esp32S3AesRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 AES register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 AES word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let mut trigger = false;
        let mut continue_operation = false;
        {
            let mut state = self.state.lock().expect("ESP32-S3 AES lock poisoned");
            if let Some(index) = register.key_index() {
                state.key[index] = value;
            } else if let Some(index) = register.text_in_index() {
                state.text_in[index] = value;
            } else if register.text_out_index().is_some() {
                return Err(DeviceError::new(
                    "ESP32-S3 AES text output registers are read-only",
                ));
            } else {
                match register {
                    Esp32S3AesRegister::Mode => state.mode = value & register.write_mask(),
                    Esp32S3AesRegister::Trigger => trigger = value & register.write_mask() != 0,
                    Esp32S3AesRegister::Continue => {
                        continue_operation = value & register.write_mask() != 0
                    }
                    Esp32S3AesRegister::IntClear => {
                        if value & register.write_mask() != 0 {
                            state.interrupt_pending = false;
                        }
                    }
                    Esp32S3AesRegister::IntEna => {
                        state.interrupt_enable = value & register.write_mask()
                    }
                    Esp32S3AesRegister::State | Esp32S3AesRegister::Date => {
                        return Err(DeviceError::new("ESP32-S3 AES register is read-only"));
                    }
                    _ => {
                        state
                            .registers
                            .insert(register, value & register.write_mask());
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

    fn write_block(device: &mut Esp32S3Aes, base: Esp32S3AesRegister, block: &[u8]) {
        let base = base.offset();
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
        write_block(&mut device, Esp32S3AesRegister::Key0, &KEY128);
        write_block(&mut device, Esp32S3AesRegister::TextIn0, &PLAINTEXT);
        device
            .write(
                Esp32S3AesRegister::IntEna.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3AesRegister::Trigger.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
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
            .write(
                Esp32S3AesRegister::IntClear.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.interrupt_pending());

        write_block(&mut device, Esp32S3AesRegister::TextIn0, &handle.text_out());
        device
            .write(
                Esp32S3AesRegister::Mode.offset(),
                AccessWidth::Word,
                AES_MODE_DECRYPT as u64,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3AesRegister::Trigger.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.text_out(), PLAINTEXT);
    }

    #[test]
    fn aes_256_and_unsupported_modes_are_explicit() {
        let hub = SignalHub::new();
        let (mut device, handle) = Esp32S3Aes::new("aes", hub).unwrap();
        write_block(&mut device, Esp32S3AesRegister::Key0, &KEY256);
        write_block(&mut device, Esp32S3AesRegister::TextIn0, &PLAINTEXT);
        device
            .write(
                Esp32S3AesRegister::Mode.offset(),
                AccessWidth::Word,
                AES_MODE_256 as u64,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3AesRegister::Trigger.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            handle.text_out(),
            [
                0xf3, 0xee, 0xd1, 0xbd, 0xb5, 0xd2, 0xa0, 0x3c, 0x06, 0x4b, 0x5a, 0x7e, 0x3d, 0xb1,
                0x81, 0xf8,
            ]
        );

        device
            .write(
                Esp32S3AesRegister::Mode.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3AesRegister::Trigger.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.text_out(), [0; 16]);
        assert!(!handle.operation_supported());
    }

    #[test]
    fn register_enum_covers_native_windows_and_rejects_invalid_access() {
        assert_eq!(Esp32S3AesRegister::Key0.offset(), 0x00);
        assert_eq!(Esp32S3AesRegister::TextOut3.offset(), 0x3c);
        assert_eq!(Esp32S3AesRegister::DmaExit.offset(), 0xb8);
        assert_eq!(Esp32S3AesRegister::from_offset(0x45), None);

        let hub = SignalHub::new();
        let (mut device, _) = Esp32S3Aes::new("aes", hub).unwrap();
        assert!(
            device
                .write(
                    Esp32S3AesRegister::Mode.offset(),
                    AccessWidth::Word,
                    1_u64 << 32,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(device.read(0x45, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            device
                .write(
                    Esp32S3AesRegister::TextOut0.offset(),
                    AccessWidth::Word,
                    0,
                    SimTime::ZERO,
                )
                .is_err()
        );
        device
            .write(
                Esp32S3AesRegister::Mode.offset(),
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3AesRegister::Mode.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            7
        );
    }
}
