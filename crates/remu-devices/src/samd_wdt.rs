use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct WdtState {
    enabled: bool,
    period_code: u8,
    started: u64,
    expired: bool,
}

/// Machine-facing SAM D21 watchdog timeout state.
#[derive(Clone)]
pub struct Samd21WdtHandle(Arc<Mutex<WdtState>>);

impl Samd21WdtHandle {
    /// Advances the functional watchdog and consumes one reset request.
    pub fn take_reset(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("WDT lock poisoned");
        let period = 8_u64 << state.period_code.min(11);
        if state.enabled && now.ticks().saturating_sub(state.started) >= period {
            state.expired = true;
            state.enabled = false;
        }
        std::mem::take(&mut state.expired)
    }
}

/// Functional SAM D21 watchdog enable/configuration/clear slice.
pub struct Samd21Wdt {
    name: String,
    state: Arc<Mutex<WdtState>>,
    interrupt_enable: bool,
    interrupt_flag: bool,
}

impl Samd21Wdt {
    /// Constructs the watchdog and reset-request handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21WdtHandle) {
        let state = Arc::new(Mutex::new(WdtState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                interrupt_enable: false,
                interrupt_flag: false,
            },
            Samd21WdtHandle(state),
        )
    }
}

impl Device for Samd21Wdt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("WDT lock poisoned");
        match offset {
            0x00 => Ok(u64::from(state.enabled) << 1),
            0x01 => Ok(u64::from(state.period_code)),
            0x04 | 0x05 => Ok(u64::from(self.interrupt_enable)),
            0x06 => Ok(u64::from(self.interrupt_flag)),
            0x07 => Ok(0),
            _ => Ok(0),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("WDT lock poisoned");
        match offset {
            0x00 => {
                state.enabled = value & 2 != 0;
                state.started = at.ticks();
                state.expired = false;
            }
            0x01 => state.period_code = (value as u8) & 0xf,
            0x04 => self.interrupt_enable &= value & 1 == 0,
            0x05 => self.interrupt_enable |= value & 1 != 0,
            0x06 => self.interrupt_flag &= value & 1 == 0,
            0x08 if value as u8 == 0xa5 => {
                state.started = at.ticks();
                state.expired = false;
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("WDT lock poisoned") = WdtState::default();
        self.interrupt_enable = false;
        self.interrupt_flag = false;
    }
}
