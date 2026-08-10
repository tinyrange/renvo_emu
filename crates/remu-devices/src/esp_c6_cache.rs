use super::*;
use std::collections::VecDeque;

struct EspC6ExtmemState {
    registers: Vec<u32>,
    syncs: VecDeque<(u32, u32)>,
}

impl EspC6ExtmemState {
    fn new() -> Self {
        let mut registers = vec![0; 0x400 / 4];
        registers[0x24 / 4] = 0x0005_0000;
        registers[0x28 / 4] = 0x0005_0000;
        registers[0x30 / 4] = 0x0003_0000;
        registers[0x34 / 4] = 0x0003_0000;
        registers[0x84 / 4] = 0x3fff_3fff;
        registers[0x88 / 4] = 1 << 2;
        registers[0x98 / 4] = 1 << 4;
        registers[0x9c / 4] = 0x3f;
        // L1_CACHE_AUTOLOAD_DONE is read-only and asserted at reset. Cache
        // ROM routines poll it after changing AUTOLOAD_ENA.
        registers[0x134 / 4] = 1 << 1;
        registers[0xd8 / 4] = 1 << 1;
        registers[0x3fc / 4] = 0x0220_2080;
        Self {
            registers,
            syncs: VecDeque::new(),
        }
    }
}

/// Scheduler-facing cache-maintenance state for the ESP32-C6.
#[derive(Clone)]
pub struct EspC6ExtmemHandle {
    state: Rc<RefCell<EspC6ExtmemState>>,
}

impl EspC6ExtmemHandle {
    /// Consumes the next completed invalidate/clean/writeback range.
    pub fn take_sync(&self) -> Option<(u32, u32)> {
        self.state.borrow_mut().syncs.pop_front()
    }

    /// Reports whether the instruction cache bus is available.
    pub fn instruction_bus_enabled(&self) -> bool {
        self.state.borrow().registers[0x04 / 4] & 1 == 0
    }

    /// Reports whether the data cache bus is available.
    pub fn data_bus_enabled(&self) -> bool {
        self.state.borrow().registers[0x04 / 4] & 2 == 0
    }

    /// Reports whether cache tag/data state is frozen.
    pub fn frozen(&self) -> bool {
        self.state.borrow().registers[0x2c / 4] & (1 << 16) != 0
    }
}

/// ESP32-C6 L1 cache controller with synchronous maintenance completion.
pub struct EspC6Extmem {
    name: String,
    state: Rc<RefCell<EspC6ExtmemState>>,
}

impl EspC6Extmem {
    /// Creates the cache register page and maintenance handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6ExtmemHandle) {
        let state = Rc::new(RefCell::new(EspC6ExtmemState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6ExtmemHandle { state },
        )
    }

    fn index(offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x400 {
            return Err(DeviceError::new(
                "ESP32-C6 EXTMEM requires an aligned word access",
            ));
        }
        Ok(offset as usize / 4)
    }
}

impl Device for EspC6Extmem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = Self::index(offset, width)?;
        Ok(u64::from(self.state.borrow().registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = Self::index(offset, width)?;
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        match offset {
            0x04 => state.registers[index] = value & 3,
            0x24 | 0x28 => state.registers[index] = value & 0x0005_0000,
            0x2c => {
                // L1 cache freeze completes synchronously. DONE is read-only
                // and follows the enable transition while MODE is retained.
                let request = value & 0x0003_0000;
                state.registers[index] = request | ((request & (1 << 16)) << 2);
            }
            0x30 | 0x34 => state.registers[index] = value & 0x0003_ffff,
            0x84 => state.registers[index] = value & 0x3fff_3fff,
            0x88 => {
                if value & 3 != 0 {
                    state.registers[index] = 1 << 2;
                }
            }
            0x98 => {
                if value & 0xf != 0 {
                    let address = state.registers[0xa0 / 4];
                    let size = state.registers[0xa4 / 4] & 0x00ff_ffff;
                    state.syncs.push_back((address, size));
                    state.registers[index] = 1 << 4;
                }
            }
            0x9c => state.registers[index] = value & 0x3f,
            0xa4 => state.registers[index] = value & 0x00ff_ffff,
            0x134 => {
                // Autoload completes synchronously at this fidelity. Preserve
                // the read-only DONE bit while accepting the documented
                // enable, order, trigger-mode, and section-enable fields.
                state.registers[index] = (value & 0x0000_031d) | (1 << 1);
            }
            0xd8 => {
                state.registers[index] = (value & (1 << 2)) | (1 << 1);
            }
            0xe0 => state.registers[index] = value & 0x3fff,
            0x3fc => state.registers[index] = value & 0x0fff_ffff,
            _ => state.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspC6ExtmemState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_sync_completes_and_publishes_exact_range() {
        let (mut cache, handle) = EspC6Extmem::new("cache");
        cache
            .write(0xa0, AccessWidth::Word, 0x4280_1000, SimTime::ZERO)
            .unwrap();
        cache
            .write(0xa4, AccessWidth::Word, 0x200, SimTime::ZERO)
            .unwrap();
        cache
            .write(0x98, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            cache.read(0x98, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 4
        );
        assert_eq!(handle.take_sync(), Some((0x4280_1000, 0x200)));
        assert_eq!(handle.take_sync(), None);
    }

    #[test]
    fn cache_bus_shutdown_freeze_and_reset_defaults_are_visible() {
        let (mut cache, handle) = EspC6Extmem::new("cache");
        assert!(handle.instruction_bus_enabled());
        assert!(handle.data_bus_enabled());
        cache.write(4, AccessWidth::Word, 3, SimTime::ZERO).unwrap();
        cache
            .write(0x2c, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        assert!(!handle.instruction_bus_enabled());
        assert!(!handle.data_bus_enabled());
        assert!(handle.frozen());
        assert_eq!(
            cache.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            (1 << 18) | (1 << 16)
        );
        cache
            .write(0x2c, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert!(!handle.frozen());
        assert_eq!(
            cache.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
        cache.reset(ResetKind::Software);
        assert_eq!(
            cache.read(0x3fc, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0220_2080
        );
    }

    #[test]
    fn cache_autoload_done_survives_rom_enable_transitions() {
        let (mut cache, _) = EspC6Extmem::new("cache");
        assert_eq!(
            cache.read(0x134, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 1
        );

        cache
            .write(0x134, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            cache.read(0x134, AccessWidth::Word, SimTime::ZERO).unwrap(),
            3
        );
        cache
            .write(0x134, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            cache.read(0x134, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 1
        );
    }
}
