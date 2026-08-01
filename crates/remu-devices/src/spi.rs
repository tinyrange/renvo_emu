use super::*;
use std::collections::VecDeque;

/// Host-facing state for a functional SPI controller.
#[derive(Clone, Default)]
pub struct SpiHandle {
    transmitted: Arc<Mutex<Vec<u8>>>,
    received: Arc<Mutex<VecDeque<u8>>>,
}

impl SpiHandle {
    /// Returns bytes written to the controller's data register.
    pub fn transmitted(&self) -> Vec<u8> {
        self.transmitted
            .lock()
            .expect("SPI transmit lock poisoned")
            .clone()
    }

    /// Queues bytes to be returned by subsequent data-register reads.
    pub fn queue_received(&self, bytes: &[u8]) {
        self.received
            .lock()
            .expect("SPI receive lock poisoned")
            .extend(bytes.iter().copied());
    }

    /// Clears captured traffic and pending receive bytes.
    pub fn clear(&self) {
        self.transmitted
            .lock()
            .expect("SPI transmit lock poisoned")
            .clear();
        self.received
            .lock()
            .expect("SPI receive lock poisoned")
            .clear();
    }
}

/// Deterministic byte-oriented SPI controller.
///
/// The register layout follows the ARM PL022-style controller used by the RP2040 and RP2350:
/// `CR0` at `0x00`, `CR1` at `0x04`, `DR` at `0x08`, `SR` at `0x0c`, `CPSR` at `0x10`, and the
/// interrupt/DMA registers from `0x14` through `0x24`. Each data write records one transmitted
/// byte and produces one received byte. A queued host byte is consumed first; otherwise the
/// transmitted byte is looped back, giving firmware tests a useful deterministic endpoint.
pub struct FunctionalSpi {
    name: String,
    registers: [u32; 10],
    handle: SpiHandle,
}

impl FunctionalSpi {
    /// Creates a reset SPI controller and host handle.
    pub fn new(name: impl Into<String>) -> (Self, SpiHandle) {
        let handle = SpiHandle::default();
        (
            Self {
                name: name.into(),
                registers: [0; 10],
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn check_access(offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("SPI requires aligned word access"));
        }
        Ok(offset & 0x0fff)
    }

    fn status(&self) -> u32 {
        let received = self
            .handle
            .received
            .lock()
            .expect("SPI receive lock poisoned");
        // TFE and TNF remain asserted because the functional model never blocks on a TX FIFO.
        0x03 | u32::from(!received.is_empty()) << 3 | u32::from(received.len() >= 8) << 2
    }
}

impl Device for FunctionalSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let offset = Self::check_access(offset, width)?;
        let value = match offset {
            0x08 => u32::from(
                self.handle
                    .received
                    .lock()
                    .expect("SPI receive lock poisoned")
                    .pop_front()
                    .unwrap_or(0),
            ),
            0x0c => self.status(),
            0x00..=0x04 | 0x10..=0x24 => {
                let index = usize::try_from(offset / 4).expect("SPI register index fits");
                self.registers[index]
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled SPI read at offset {offset:#x}"
                )));
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
        let offset = Self::check_access(offset, width)?;
        let byte = value.to_le_bytes()[0];
        if offset == 0x08 {
            self.handle
                .transmitted
                .lock()
                .expect("SPI transmit lock poisoned")
                .push(byte);
            let received = self
                .handle
                .received
                .lock()
                .expect("SPI receive lock poisoned")
                .pop_front()
                .unwrap_or(byte);
            self.handle
                .received
                .lock()
                .expect("SPI receive lock poisoned")
                .push_back(received);
            return Ok(());
        }
        let index = match offset {
            0x00..=0x04 | 0x10..=0x24 => {
                usize::try_from(offset / 4).expect("SPI register index fits")
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled SPI write at offset {offset:#x}"
                )));
            }
        };
        self.registers[index] =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked SPI register value fits");
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers = [0; 10];
        self.handle.clear();
    }
}
