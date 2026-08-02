use super::*;
use sha2::{Digest, Sha224, Sha256};

const MODE: u64 = 0x00;
const T_STRING: u64 = 0x04;
const T_LENGTH: u64 = 0x08;
const BLOCK_NUM: u64 = 0x0c;
const START: u64 = 0x10;
const CONTINUE: u64 = 0x14;
const BUSY: u64 = 0x18;
const DMA_START: u64 = 0x1c;
const DMA_CONTINUE: u64 = 0x20;
const CLEAR_IRQ: u64 = 0x24;
const INT_ENA: u64 = 0x28;
const H_BASE: u64 = 0x40;
const TEXT_BASE: u64 = 0x80;
const DATE: u64 = 0xfc;

const TEXT_BYTES: usize = 64;
const DIGEST_BYTES: usize = 32;

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
    registers: BTreeMap<u64, u32>,
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
        registers.insert(DATE, 0x2025_0001);
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

    fn register(&self, offset: u64) -> u32 {
        self.registers.get(&offset).copied().unwrap_or_default()
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

    fn set_digest(&mut self, message: &[u8], at: SimTime) -> Result<(), DeviceError> {
        self.digest.fill(0);
        match self.register(MODE) {
            1 => {
                let digest = Sha224::digest(message);
                self.digest[..digest.len()].copy_from_slice(&digest);
            }
            2 => {
                let digest = Sha256::digest(message);
                self.digest.copy_from_slice(&digest);
            }
            _ => {
                // SHA1/SHA384/SHA512 are left explicit rather than silently
                // returning a SHA-256 value for the wrong mode.
            }
        }
        self.irq = true;
        self.registers.insert(BUSY, 0);
        self.publish(at)
    }

    fn process_text(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let bytes = self.text;
        self.set_digest(&bytes, at)
    }

    fn process_dma(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let message = std::mem::take(&mut self.dma_message);
        self.set_digest(&message, at)
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.registers.insert(MODE, 2);
        self.registers.insert(T_STRING, 0);
        self.registers.insert(T_LENGTH, 0);
        self.registers.insert(BLOCK_NUM, 0);
        self.registers.insert(DATE, 0x2025_0001);
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
        let state = self.state.borrow();
        let value = match offset {
            BUSY => state.register(BUSY),
            INT_ENA => state.register(INT_ENA),
            H_BASE..=0x5c if (offset - H_BASE) % 4 == 0 => {
                let index = usize::try_from((offset - H_BASE) / 4).expect("SHA digest index fits");
                let start = index * 4;
                u32::from_be_bytes(
                    state.digest[start..start + 4]
                        .try_into()
                        .expect("digest word"),
                )
            }
            TEXT_BASE..=0xbc if (offset - TEXT_BASE) % 4 == 0 => {
                let index = usize::try_from((offset - TEXT_BASE) / 4).expect("SHA text index fits");
                u32::from_le_bytes(
                    state.text[index * 4..index * 4 + 4]
                        .try_into()
                        .expect("text word"),
                )
            }
            _ => state.register(offset),
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
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            TEXT_BASE..=0xbc if (offset - TEXT_BASE) % 4 == 0 => {
                let index = usize::try_from((offset - TEXT_BASE) / 4).expect("SHA text index fits");
                state.text[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            H_BASE..=0x5c if (offset - H_BASE) % 4 == 0 => {
                let index = usize::try_from((offset - H_BASE) / 4).expect("SHA digest index fits");
                state.digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
            }
            START | CONTINUE => {
                state.registers.insert(offset, value);
                if value != 0 {
                    state.registers.insert(BUSY, 1);
                    state.process_text(at)?;
                }
            }
            DMA_START | DMA_CONTINUE => {
                state.registers.insert(offset, value);
                if value != 0 {
                    state.registers.insert(BUSY, 1);
                    state.process_dma(at)?;
                }
            }
            CLEAR_IRQ => {
                state.irq = false;
                state.registers.insert(CLEAR_IRQ, 0);
            }
            _ => {
                state.registers.insert(offset, value);
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
        sha.write(INT_ENA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.queue_dma_message(b"abc");
        sha.write(DMA_START, AccessWidth::Word, 1, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(handle.digest(), Sha256::digest(b"abc").to_vec());
        assert_eq!(
            sha.read(H_BASE, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xba78_16bf
        );
        assert!(handle.interrupt_pending());
        sha.write(CLEAR_IRQ, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());

        sha.write(MODE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.queue_dma_message(b"abc");
        sha.write(DMA_START, AccessWidth::Word, 1, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(handle.digest(), Sha224::digest(b"abc").to_vec());
    }

    #[test]
    fn text_window_is_little_endian_and_unsupported_modes_are_explicit() {
        let hub = SignalHub::new();
        let (mut sha, handle) = Esp32S3Sha::new("sha", hub).unwrap();
        sha.write(TEXT_BASE, AccessWidth::Word, 0x64636261, SimTime::ZERO)
            .unwrap();
        sha.write(START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            sha.read(TEXT_BASE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x6463_6261
        );
        assert_ne!(handle.digest(), vec![0; DIGEST_BYTES]);
        sha.write(MODE, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        sha.write(START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.digest(), vec![0; DIGEST_BYTES]);
    }
}
