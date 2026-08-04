//! ESP32-S3 APB peripheral backup DMA controller.

use super::*;

const CONFIG_RESET: u32 = 0x0000_6480;
const CONFIG_WRITE_MASK: u32 = 0xffff_fff8;
const ENABLE: u32 = 1 << 31;
const TO_MEMORY: u32 = 1 << 30;
const START: u32 = 1 << 29;
const MAP_MODE: u32 = 1 << 3;
const DATE_RESET: u32 = 0x0201_2300;

/// One bus-copy request emitted by the peripheral after START.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Esp32S3PeriBackupRequest {
    /// `true` copies APB registers to memory; `false` restores them.
    pub to_memory: bool,
    /// First APB register address.
    pub apb_address: u32,
    /// First retention-memory address.
    pub memory_address: u32,
    /// Selected source word offsets, in words rather than bytes.
    pub word_offsets: Vec<u16>,
}

struct PeriBackupState {
    config: u32,
    apb_address: u32,
    memory_address: u32,
    maps: [u32; 4],
    int_raw: u32,
    int_enable: u32,
    date: u32,
    pending: Option<Esp32S3PeriBackupRequest>,
}

impl PeriBackupState {
    fn reset() -> Self {
        Self {
            config: CONFIG_RESET,
            apb_address: 0,
            memory_address: 0,
            maps: [0; 4],
            int_raw: 0,
            int_enable: 0,
            date: DATE_RESET,
            pending: None,
        }
    }

    fn start(&mut self) {
        if self.config & ENABLE == 0 {
            self.config = self.config & !7 | 1;
            self.int_raw |= 2;
            return;
        }
        if self.apb_address & 3 != 0 || self.memory_address & 3 != 0 {
            self.config = self.config & !7 | 2;
            self.int_raw |= 2;
            return;
        }
        let size = ((self.config >> 19) & 0x3ff) as usize;
        let word_offsets = if self.config & MAP_MODE == 0 {
            (0..size.min(1024)).map(|word| word as u16).collect()
        } else {
            (0..size.min(128))
                .filter(|word| self.maps[word / 32] & (1 << (word % 32)) != 0)
                .map(|word| word as u16)
                .collect()
        };
        self.config &= !7;
        self.pending = Some(Esp32S3PeriBackupRequest {
            to_memory: self.config & TO_MEMORY != 0,
            apb_address: self.apb_address,
            memory_address: self.memory_address,
            word_offsets,
        });
    }
}

/// Machine-facing transfer and interrupt handle.
#[derive(Clone)]
pub struct Esp32S3PeriBackupHandle {
    state: Rc<RefCell<PeriBackupState>>,
}

impl Esp32S3PeriBackupHandle {
    /// Takes the pending transfer request, if software issued START.
    pub fn take_request(&self) -> Option<Esp32S3PeriBackupRequest> {
        self.state.borrow_mut().pending.take()
    }

    /// Completes a request; `None` means success and `Some` records FLOW_ERR.
    pub fn complete(&self, error: Option<u8>) {
        let mut state = self.state.borrow_mut();
        if let Some(error) = error {
            state.config = state.config & !7 | u32::from(error.min(7));
            state.int_raw |= 2;
        } else {
            state.int_raw |= 1;
        }
    }

    /// Returns the enabled DONE/ERR interrupt level.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.borrow();
        state.int_raw & state.int_enable & 3 != 0
    }
}

/// Functional ESP32-S3 peripheral-backup controller.
pub struct Esp32S3PeriBackup {
    name: String,
    state: Rc<RefCell<PeriBackupState>>,
}

impl Esp32S3PeriBackup {
    /// Creates reset state and its machine-facing transfer handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3PeriBackupHandle) {
        let state = Rc::new(RefCell::new(PeriBackupState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3PeriBackupHandle { state },
        )
    }

    fn check(&self, offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 PERI_BACKUP requires aligned word access",
            ));
        }
        if !matches!(offset, 0x00..=0x28 | 0xfc) {
            return Err(DeviceError::new(format!(
                "{} access at reserved offset {offset:#x}",
                self.name
            )));
        }
        Ok(())
    }
}

impl Device for Esp32S3PeriBackup {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        self.check(offset, width)?;
        let state = self.state.borrow();
        let value = match offset {
            0x00 => state.config,
            0x04 => state.apb_address,
            0x08 => state.memory_address,
            0x0c..=0x18 => state.maps[(offset as usize - 0x0c) / 4],
            0x1c => state.int_raw,
            0x20 => state.int_raw & state.int_enable,
            0x24 => state.int_enable,
            0x28 => 0,
            0xfc => state.date,
            _ => unreachable!(),
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
        self.check(offset, width)?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 PERI_BACKUP word exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => {
                state.config = state.config & 7 | value & CONFIG_WRITE_MASK & !START;
                if value & START != 0 {
                    state.start();
                }
            }
            0x04 => state.apb_address = value,
            0x08 => state.memory_address = value,
            0x0c..=0x18 => state.maps[(offset as usize - 0x0c) / 4] = value,
            0x1c => state.int_raw &= !(value & 3),
            0x20 => {}
            0x24 => state.int_enable = value & 3,
            0x28 => state.int_raw &= !(value & 3),
            0xfc => state.date = value & 0x8fff_ffff,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = PeriBackupState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(device: &mut Esp32S3PeriBackup, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, value.into(), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn register_contract_start_completion_and_interrupts_are_functional() {
        let (mut device, handle) = Esp32S3PeriBackup::new("backup");
        assert_eq!(
            device.read(0, AccessWidth::Word, SimTime::ZERO),
            Ok(CONFIG_RESET.into())
        );
        assert_eq!(
            device.read(0xfc, AccessWidth::Word, SimTime::ZERO),
            Ok(DATE_RESET.into())
        );
        write(&mut device, 0x04, 0x6000_0000);
        write(&mut device, 0x08, 0x3fc8_8000);
        write(&mut device, 0x24, 1);
        write(&mut device, 0, ENABLE | TO_MEMORY | START | (2 << 19));
        let request = handle.take_request().unwrap();
        assert_eq!(request.word_offsets, vec![0, 1]);
        handle.complete(None);
        assert!(handle.interrupt_pending());
        write(&mut device, 0x28, 1);
        assert!(!handle.interrupt_pending());
        assert!(device.read(0x2c, AccessWidth::Word, SimTime::ZERO).is_err());
    }

    #[test]
    fn disabled_misaligned_and_sparse_requests_report_deterministically() {
        let (mut device, handle) = Esp32S3PeriBackup::new("backup");
        write(&mut device, 0, START);
        assert!(handle.take_request().is_none());
        assert_eq!(
            device.read(0, AccessWidth::Word, SimTime::ZERO).unwrap() & 7,
            1
        );
        write(&mut device, 0x04, 2);
        write(&mut device, 0, ENABLE | START);
        assert_eq!(
            device.read(0, AccessWidth::Word, SimTime::ZERO).unwrap() & 7,
            2
        );
        write(&mut device, 0x04, 0x6000_0000);
        write(&mut device, 0x0c, 0b1010);
        write(&mut device, 0, ENABLE | MAP_MODE | START | (4 << 19));
        assert_eq!(handle.take_request().unwrap().word_offsets, vec![1, 3]);
    }
}
