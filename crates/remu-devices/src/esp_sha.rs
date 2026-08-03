use super::*;
use sha2::{Digest, Sha224, Sha256};

/// Native ESP32-S3 SHA register identifiers from Espressif's
/// `hwcrypto_reg.h` map.  The digest and text windows are represented as
/// individual IDs so callers never need to use an integer register index.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Esp32S3ShaRegister {
    Mode = 0x00,
    TString = 0x04,
    TLength = 0x08,
    BlockNum = 0x0c,
    Start = 0x10,
    Continue = 0x14,
    Busy = 0x18,
    DmaStart = 0x1c,
    DmaContinue = 0x20,
    ClearIrq = 0x24,
    IntEna = 0x28,
    H0 = 0x40,
    H1 = 0x44,
    H2 = 0x48,
    H3 = 0x4c,
    H4 = 0x50,
    H5 = 0x54,
    H6 = 0x58,
    H7 = 0x5c,
    H8 = 0x60,
    H9 = 0x64,
    H10 = 0x68,
    H11 = 0x6c,
    H12 = 0x70,
    H13 = 0x74,
    H14 = 0x78,
    H15 = 0x7c,
    Text0 = 0x80,
    Text1 = 0x84,
    Text2 = 0x88,
    Text3 = 0x8c,
    Text4 = 0x90,
    Text5 = 0x94,
    Text6 = 0x98,
    Text7 = 0x9c,
    Text8 = 0xa0,
    Text9 = 0xa4,
    Text10 = 0xa8,
    Text11 = 0xac,
    Text12 = 0xb0,
    Text13 = 0xb4,
    Text14 = 0xb8,
    Text15 = 0xbc,
    Text16 = 0xc0,
    Text17 = 0xc4,
    Text18 = 0xc8,
    Text19 = 0xcc,
    Text20 = 0xd0,
    Text21 = 0xd4,
    Text22 = 0xd8,
    Text23 = 0xdc,
    Text24 = 0xe0,
    Text25 = 0xe4,
    Text26 = 0xe8,
    Text27 = 0xec,
    Text28 = 0xf0,
    Text29 = 0xf4,
    Text30 = 0xf8,
    Text31 = 0xfc,
    Date = 0x2c,
}

impl Esp32S3ShaRegister {
    /// Returns the byte offset in the SHA peripheral page.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Resolves a native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Mode,
            0x04 => Self::TString,
            0x08 => Self::TLength,
            0x0c => Self::BlockNum,
            0x10 => Self::Start,
            0x14 => Self::Continue,
            0x18 => Self::Busy,
            0x1c => Self::DmaStart,
            0x20 => Self::DmaContinue,
            0x24 => Self::ClearIrq,
            0x28 => Self::IntEna,
            0x40 => Self::H0,
            0x44 => Self::H1,
            0x48 => Self::H2,
            0x4c => Self::H3,
            0x50 => Self::H4,
            0x54 => Self::H5,
            0x58 => Self::H6,
            0x5c => Self::H7,
            0x60 => Self::H8,
            0x64 => Self::H9,
            0x68 => Self::H10,
            0x6c => Self::H11,
            0x70 => Self::H12,
            0x74 => Self::H13,
            0x78 => Self::H14,
            0x7c => Self::H15,
            0x80 => Self::Text0,
            0x84 => Self::Text1,
            0x88 => Self::Text2,
            0x8c => Self::Text3,
            0x90 => Self::Text4,
            0x94 => Self::Text5,
            0x98 => Self::Text6,
            0x9c => Self::Text7,
            0xa0 => Self::Text8,
            0xa4 => Self::Text9,
            0xa8 => Self::Text10,
            0xac => Self::Text11,
            0xb0 => Self::Text12,
            0xb4 => Self::Text13,
            0xb8 => Self::Text14,
            0xbc => Self::Text15,
            0xc0 => Self::Text16,
            0xc4 => Self::Text17,
            0xc8 => Self::Text18,
            0xcc => Self::Text19,
            0xd0 => Self::Text20,
            0xd4 => Self::Text21,
            0xd8 => Self::Text22,
            0xdc => Self::Text23,
            0xe0 => Self::Text24,
            0xe4 => Self::Text25,
            0xe8 => Self::Text26,
            0xec => Self::Text27,
            0xf0 => Self::Text28,
            0xf4 => Self::Text29,
            0xf8 => Self::Text30,
            0xfc => Self::Text31,
            0x2c => Self::Date,
            _ => return None,
        })
    }

    fn digest_index(self) -> Option<usize> {
        Some(match self {
            Self::H0 => 0,
            Self::H1 => 1,
            Self::H2 => 2,
            Self::H3 => 3,
            Self::H4 => 4,
            Self::H5 => 5,
            Self::H6 => 6,
            Self::H7 => 7,
            Self::H8 => 8,
            Self::H9 => 9,
            Self::H10 => 10,
            Self::H11 => 11,
            Self::H12 => 12,
            Self::H13 => 13,
            Self::H14 => 14,
            Self::H15 => 15,
            _ => return None,
        })
    }

    fn text_index(self) -> Option<usize> {
        Some(match self {
            Self::Text0 => 0,
            Self::Text1 => 1,
            Self::Text2 => 2,
            Self::Text3 => 3,
            Self::Text4 => 4,
            Self::Text5 => 5,
            Self::Text6 => 6,
            Self::Text7 => 7,
            Self::Text8 => 8,
            Self::Text9 => 9,
            Self::Text10 => 10,
            Self::Text11 => 11,
            Self::Text12 => 12,
            Self::Text13 => 13,
            Self::Text14 => 14,
            Self::Text15 => 15,
            Self::Text16 => 16,
            Self::Text17 => 17,
            Self::Text18 => 18,
            Self::Text19 => 19,
            Self::Text20 => 20,
            Self::Text21 => 21,
            Self::Text22 => 22,
            Self::Text23 => 23,
            Self::Text24 => 24,
            Self::Text25 => 25,
            Self::Text26 => 26,
            Self::Text27 => 27,
            Self::Text28 => 28,
            Self::Text29 => 29,
            Self::Text30 => 30,
            Self::Text31 => 31,
            _ => return None,
        })
    }

    const fn read_mask(self) -> u32 {
        match self {
            Self::Busy => 1,
            Self::Mode => 0x7,
            Self::TLength | Self::BlockNum => 0x3f,
            Self::IntEna => 1,
            Self::Start | Self::Continue | Self::DmaStart | Self::DmaContinue | Self::ClearIrq => 0,
            Self::Date => u32::MAX,
            _ => u32::MAX,
        }
    }

    const fn write_mask(self) -> u32 {
        match self {
            Self::Busy | Self::Date => 0,
            Self::Mode => 0x7,
            Self::TLength | Self::BlockNum => 0x3f,
            Self::IntEna => 1,
            Self::Start | Self::Continue | Self::DmaStart | Self::DmaContinue | Self::ClearIrq => 1,
            _ => u32::MAX,
        }
    }
}

const MODE: Esp32S3ShaRegister = Esp32S3ShaRegister::Mode;
const T_STRING: Esp32S3ShaRegister = Esp32S3ShaRegister::TString;
const T_LENGTH: Esp32S3ShaRegister = Esp32S3ShaRegister::TLength;
const BLOCK_NUM: Esp32S3ShaRegister = Esp32S3ShaRegister::BlockNum;
const START: Esp32S3ShaRegister = Esp32S3ShaRegister::Start;
const CONTINUE: Esp32S3ShaRegister = Esp32S3ShaRegister::Continue;
const BUSY: Esp32S3ShaRegister = Esp32S3ShaRegister::Busy;
const DMA_START: Esp32S3ShaRegister = Esp32S3ShaRegister::DmaStart;
const DMA_CONTINUE: Esp32S3ShaRegister = Esp32S3ShaRegister::DmaContinue;
const CLEAR_IRQ: Esp32S3ShaRegister = Esp32S3ShaRegister::ClearIrq;
const INT_ENA: Esp32S3ShaRegister = Esp32S3ShaRegister::IntEna;
const DATE: Esp32S3ShaRegister = Esp32S3ShaRegister::Date;

const TEXT_BYTES: usize = 128;
const DIGEST_BYTES: usize = 32;
const SHA_DATE_RESET: u32 = 0x2019_0402;

/// Host-side input/output endpoint for the ESP32-S3 SHA accelerator.
#[derive(Clone)]
pub struct Esp32S3ShaHandle {
    state: Rc<RefCell<Esp32S3ShaState>>,
}

impl Esp32S3ShaHandle {
    /// Queues a complete message for the functional DMA start path.
    pub fn queue_dma_message(&self, message: impl AsRef<[u8]>) {
        self.state
            .borrow_mut()
            .dma_message
            .extend_from_slice(message.as_ref());
    }

    /// Returns the most recently completed digest in big-endian byte order.
    pub fn digest(&self) -> Vec<u8> {
        let state = self.state.borrow();
        let length = if state.register(MODE) == 1 {
            28
        } else {
            DIGEST_BYTES
        };
        state.digest[..length].to_vec()
    }

    /// Reports whether an enabled SHA completion interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.borrow();
        state.irq && state.register(INT_ENA) != 0
    }
}

struct Esp32S3ShaState {
    registers: BTreeMap<Esp32S3ShaRegister, u32>,
    text: [u8; TEXT_BYTES],
    digest: [u8; DIGEST_BYTES],
    dma_message: Vec<u8>,
    irq: bool,
    hub: SignalHub,
    digest_signal: SignalId,
}

impl Esp32S3ShaState {
    fn new(hub: SignalHub, digest_signal: SignalId) -> Self {
        let mut registers = BTreeMap::new();
        registers.insert(MODE, 2);
        registers.insert(T_STRING, 0);
        registers.insert(T_LENGTH, 0);
        registers.insert(BLOCK_NUM, 0);
        registers.insert(DATE, SHA_DATE_RESET);
        Self {
            registers,
            text: [0; TEXT_BYTES],
            digest: [0; DIGEST_BYTES],
            dma_message: Vec::new(),
            irq: false,
            hub,
            digest_signal,
        }
    }

    fn register(&self, register: Esp32S3ShaRegister) -> u32 {
        self.registers.get(&register).copied().unwrap_or_default()
    }

    fn publish(&self, at: SimTime) -> Result<(), DeviceError> {
        let word = u32::from_be_bytes(self.digest[..4].try_into().expect("digest has four bytes"));
        self.hub
            .set(
                self.digest_signal,
                SignalValue::from_u64(u64::from(word), 32)
                    .expect("fixed SHA signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn validate_mode(&self) -> Result<u32, DeviceError> {
        let mode = self.register(MODE);
        match mode {
            1 | 2 => Ok(mode),
            _ => Err(DeviceError::new(format!(
                "ESP32-S3 SHA mode {mode} is not implemented by the functional model"
            ))),
        }
    }

    fn set_digest(&mut self, message: &[u8], at: SimTime, dma: bool) -> Result<(), DeviceError> {
        let mode = self.validate_mode()?;
        self.digest.fill(0);
        match mode {
            1 => {
                let digest = Sha224::digest(message);
                self.digest[..digest.len()].copy_from_slice(&digest);
            }
            2 => {
                let digest = Sha256::digest(message);
                self.digest.copy_from_slice(&digest);
            }
            _ => unreachable!("validate_mode only accepts SHA-224 and SHA-256"),
        }
        // The native interrupt is available only to DMA-SHA. Typical SHA
        // completion is observed by polling SHA_BUSY_REG.
        self.irq = dma;
        self.registers.insert(BUSY, 0);
        self.publish(at)
    }

    fn process_text(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let bytes = self.text[..64].to_vec();
        self.set_digest(&bytes, at, false)
    }

    fn process_dma(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let message = std::mem::take(&mut self.dma_message);
        self.set_digest(&message, at, true)
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.registers.insert(MODE, 2);
        self.registers.insert(T_STRING, 0);
        self.registers.insert(T_LENGTH, 0);
        self.registers.insert(BLOCK_NUM, 0);
        self.registers.insert(DATE, SHA_DATE_RESET);
        self.text.fill(0);
        self.digest.fill(0);
        self.dma_message.clear();
        self.irq = false;
    }
}

/// Functional ESP32-S3 SHA accelerator.
///
/// The native register layout supports SHA-224 and SHA-256 text-block and
/// host-backed DMA operations synchronously, exposing digest words, busy and
/// completion-interrupt state. SHA-1/SHA-384/SHA-512, exact block scheduling,
/// DMA descriptor execution, and cycle timing remain outside this model.
pub struct Esp32S3Sha {
    name: String,
    state: Rc<RefCell<Esp32S3ShaState>>,
}

impl Esp32S3Sha {
    /// Creates the SHA register page and host endpoint.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3ShaHandle), SignalError> {
        let digest_signal = hub.declare(
            "board.esp32s3.sha.digest",
            SignalValue::from_u64(0, 32)?,
            Some("ESP32-S3 SHA digest first word".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(Esp32S3ShaState::new(hub, digest_signal)));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3ShaHandle { state },
        ))
    }
}

impl Device for Esp32S3Sha {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SHA requires aligned word access",
            ));
        }
        let register = Esp32S3ShaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 SHA register offset {offset:#x}"
            ))
        })?;
        let state = self.state.borrow();
        let value = match register {
            register if register.digest_index().is_some() => {
                let index = register.digest_index().expect("matched digest register");
                let start = index * 4;
                u32::from_be_bytes(
                    state.digest[start..start + 4]
                        .try_into()
                        .expect("digest word"),
                )
            }
            register if register.text_index().is_some() => {
                let index = register.text_index().expect("matched text register");
                u32::from_le_bytes(
                    state.text[index * 4..index * 4 + 4]
                        .try_into()
                        .expect("text word"),
                )
            }
            _ => state.register(register) & register.read_mask(),
        };
        Ok(u64::from(value))
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
                "ESP32-S3 SHA requires aligned word access",
            ));
        }
        let register = Esp32S3ShaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 SHA register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 SHA word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let mut state = self.state.borrow_mut();
        match register {
            register if register.text_index().is_some() => {
                let index = register.text_index().expect("matched text register");
                state.text[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            register if register.digest_index().is_some() => {
                let index = register.digest_index().expect("matched digest register");
                state.digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
            }
            START | CONTINUE => {
                if value & register.write_mask() != 0 {
                    state.validate_mode()?;
                    state.registers.insert(BUSY, 1);
                    state.process_text(at)?;
                }
            }
            DMA_START | DMA_CONTINUE => {
                if value & register.write_mask() != 0 {
                    state.validate_mode()?;
                    state.registers.insert(BUSY, 1);
                    state.process_dma(at)?;
                }
            }
            CLEAR_IRQ => {
                if value & register.write_mask() != 0 {
                    state.irq = false;
                }
            }
            _ => {
                state
                    .registers
                    .insert(register, value & register.write_mask());
            }
        }
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
    fn dma_sha256_and_sha224_digest_words_are_native() {
        let hub = SignalHub::new();
        let (mut sha, handle) = Esp32S3Sha::new("sha", hub).unwrap();
        sha.write(INT_ENA.offset(), AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.queue_dma_message(b"abc");
        sha.write(
            DMA_START.offset(),
            AccessWidth::Word,
            1,
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert_eq!(handle.digest(), Sha256::digest(b"abc").to_vec());
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::H0.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0xba78_16bf
        );
        assert!(handle.interrupt_pending());
        sha.write(CLEAR_IRQ.offset(), AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());

        sha.write(MODE.offset(), AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.queue_dma_message(b"abc");
        sha.write(
            DMA_START.offset(),
            AccessWidth::Word,
            1,
            SimTime::from_ticks(3),
        )
        .unwrap();
        assert_eq!(handle.digest(), Sha224::digest(b"abc").to_vec());
    }

    #[test]
    fn text_window_is_little_endian_and_unsupported_modes_are_explicit() {
        let hub = SignalHub::new();
        let (mut sha, handle) = Esp32S3Sha::new("sha", hub).unwrap();
        sha.write(INT_ENA.offset(), AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        sha.write(
            Esp32S3ShaRegister::Text0.offset(),
            AccessWidth::Word,
            0x64636261,
            SimTime::ZERO,
        )
        .unwrap();
        sha.write(START.offset(), AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::Text0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x6463_6261
        );
        assert_ne!(handle.digest(), vec![0; DIGEST_BYTES]);
        sha.write(MODE.offset(), AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        assert!(
            sha.write(START.offset(), AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
        assert_eq!(
            sha.read(BUSY.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn register_enum_enforces_native_masks_and_self_clearing_strobes() {
        assert_eq!(Esp32S3ShaRegister::Mode.offset(), 0x00);
        assert_eq!(Esp32S3ShaRegister::H15.offset(), 0x7c);
        assert_eq!(Esp32S3ShaRegister::Text31.offset(), 0xfc);
        assert_eq!(Esp32S3ShaRegister::Date.offset(), 0x2c);
        assert_eq!(Esp32S3ShaRegister::from_offset(0x30), None);

        let hub = SignalHub::new();
        let (mut sha, _) = Esp32S3Sha::new("sha", hub).unwrap();
        sha.write(
            Esp32S3ShaRegister::Mode.offset(),
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::Mode.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            7
        );
        sha.write(
            Esp32S3ShaRegister::TLength.offset(),
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::TLength.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x3f
        );
        sha.write(
            Esp32S3ShaRegister::BlockNum.offset(),
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::BlockNum.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x3f
        );
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::Date.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            u64::from(SHA_DATE_RESET)
        );
        sha.write(
            Esp32S3ShaRegister::Mode.offset(),
            AccessWidth::Word,
            2,
            SimTime::ZERO,
        )
        .unwrap();
        sha.write(
            Esp32S3ShaRegister::Start.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::Start.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
        sha.write(
            Esp32S3ShaRegister::Text31.offset(),
            AccessWidth::Word,
            0xa5a5_5a5a,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            sha.read(
                Esp32S3ShaRegister::Text31.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0xa5a5_5a5a
        );
    }

    #[test]
    fn register_access_rejects_reserved_offsets_and_wide_words() {
        let hub = SignalHub::new();
        let (mut sha, _) = Esp32S3Sha::new("sha", hub).unwrap();
        assert!(sha.read(0x30, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            sha.write(
                Esp32S3ShaRegister::Mode.offset(),
                AccessWidth::Word,
                1_u64 << 32,
                SimTime::ZERO,
            )
            .is_err()
        );
    }
}
