use super::*;

const ESP_INTERRUPT_SOURCE_COUNT: usize = 128;
const ESP_INTERRUPT_CORE1_BASE: u64 = 0x800;
const ESP_INTERRUPT_ROUTE_DISABLED: u8 = u8::MAX;

struct EspInterruptMatrixState {
    routes: [[u8; ESP_INTERRUPT_SOURCE_COUNT]; 2],
}

impl EspInterruptMatrixState {
    fn new() -> Self {
        Self {
            // ESP-IDF's reset value is interrupt line 16. The host-facing
            // handle translates the native disabled value 0x1f to u8::MAX.
            routes: [[16; ESP_INTERRUPT_SOURCE_COUNT]; 2],
        }
    }

    fn route(&self, core: usize, source: usize) -> u8 {
        self.routes
            .get(core)
            .and_then(|routes| routes.get(source))
            .copied()
            .unwrap_or(ESP_INTERRUPT_ROUTE_DISABLED)
    }

    fn set_route(&mut self, core: usize, source: usize, interrupt: u8) {
        if let Some(route) = self
            .routes
            .get_mut(core)
            .and_then(|routes| routes.get_mut(source))
        {
            *route = if interrupt & 0x1f == 0x1f {
                ESP_INTERRUPT_ROUTE_DISABLED
            } else {
                interrupt & 0x1f
            };
        }
    }
}

/// Scheduler-facing ESP32-S3 interrupt-matrix route view.
#[derive(Clone)]
pub struct EspInterruptMatrixHandle {
    state: Rc<RefCell<EspInterruptMatrixState>>,
}

impl EspInterruptMatrixHandle {
    /// Returns the CPU interrupt line for a source, or `u8::MAX` when disabled.
    pub fn route(&self, core: usize, source: usize) -> u8 {
        self.state.borrow().route(core, source)
    }

    /// Updates one CPU/source route using the native five-bit encoding.
    pub fn set_route(&self, core: usize, source: usize, interrupt: u32) {
        self.state.borrow_mut().set_route(
            core,
            source,
            u8::try_from(interrupt & 0x1f).unwrap_or(0x1f),
        );
    }
}

/// Functional ESP32-S3 CPU0/CPU1 interrupt-matrix register block.
///
/// CPU0 source routes occupy `0x000..0x1fc`; CPU1 routes occupy
/// `0x800..0x9fc`, matching the official `interrupt_core0_reg.h` and
/// `interrupt_core1_reg.h` layouts. Routing is deterministic and functional;
/// priority, NMI timing, and cycle-accurate SMP arbitration remain outside
/// this model.
pub struct EspInterruptMatrix {
    name: String,
    state: Rc<RefCell<EspInterruptMatrixState>>,
}

impl EspInterruptMatrix {
    /// Creates a reset interrupt matrix and scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspInterruptMatrixHandle) {
        let state = Rc::new(RefCell::new(EspInterruptMatrixState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspInterruptMatrixHandle { state },
        )
    }

    fn decode(offset: u64) -> Option<(usize, usize)> {
        let (core, relative) = if offset < ESP_INTERRUPT_CORE1_BASE {
            (0, offset)
        } else if offset < ESP_INTERRUPT_CORE1_BASE + 0x200 {
            (1, offset - ESP_INTERRUPT_CORE1_BASE)
        } else {
            return None;
        };
        (relative & 3 == 0).then_some((core, (relative / 4) as usize))
    }
}

impl Device for EspInterruptMatrix {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP interrupt matrix requires aligned word access",
            ));
        }
        let (core, source) = Self::decode(offset)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        Ok(u64::from(self.state.borrow().route(core, source).min(0x1f)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP interrupt matrix requires aligned word access",
            ));
        }
        let (core, source) = Self::decode(offset)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        self.state.borrow_mut().set_route(core, source, value as u8);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspInterruptMatrixState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_both_native_core_route_windows_and_disabled_encoding() {
        let (mut device, handle) = EspInterruptMatrix::new("interrupt-matrix");
        device
            .write(38 * 4, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        device
            .write(
                ESP_INTERRUPT_CORE1_BASE + 39 * 4,
                AccessWidth::Word,
                7,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.route(0, 38), 5);
        assert_eq!(handle.route(1, 39), 7);
        assert_eq!(device.read(38 * 4, AccessWidth::Word, SimTime::ZERO), Ok(5));
        device
            .write(38 * 4, AccessWidth::Word, 0x1f, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.route(0, 38), u8::MAX);
        assert_eq!(
            device.read(38 * 4, AccessWidth::Word, SimTime::ZERO),
            Ok(0x1f)
        );
    }
}
