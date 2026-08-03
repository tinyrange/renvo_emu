use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct IwdgState {
    enabled: bool,
    unlocked: bool,
    pending: bool,
    started: u64,
    prescaler: u8,
    reload: u16,
}

const IWDG_KEY_START: u32 = 0xcccc;
const IWDG_KEY_REFRESH: u32 = 0xaaaa;
const IWDG_KEY_UNLOCK: u32 = 0x5555;
const IWDG_PR_MASK: u32 = 0x07;
const IWDG_RLR_MASK: u32 = 0x0fff;

fn iwdg_prescaler_divider(value: u8) -> u64 {
    match value & 0x07 {
        0 => 4,
        1 => 8,
        2 => 16,
        3 => 32,
        4 => 64,
        5 => 128,
        _ => 256,
    }
}

/// Machine handle for STM32 independent-watchdog reset requests.
#[derive(Clone)]
pub struct Stm32WatchdogHandle(Arc<Mutex<IwdgState>>);

impl Stm32WatchdogHandle {
    /// Advances the functional watchdog and consumes one reset request.
    pub fn take_reset(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("IWDG lock poisoned");
        let period =
            (u64::from(state.reload) + 1).saturating_mul(iwdg_prescaler_divider(state.prescaler));
        if state.enabled && now.ticks().saturating_sub(state.started) >= period {
            state.pending = true;
            state.started = now.ticks();
        }
        std::mem::take(&mut state.pending)
    }
}

/// Functional STM32L4 independent-watchdog key, reload, and timeout slice.
pub struct Stm32Watchdog {
    name: String,
    state: Arc<Mutex<IwdgState>>,
    registers: [u32; 4],
}

impl Stm32Watchdog {
    /// Constructs an IWDG and its machine reset handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32WatchdogHandle) {
        let state = Arc::new(Mutex::new(IwdgState {
            reload: 0xfff,
            ..IwdgState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 4],
            },
            Stm32WatchdogHandle(state),
        )
    }
}

impl Device for Stm32Watchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 IWDG requires word accesses"));
        }
        let state = self.state.lock().expect("IWDG lock poisoned");
        let value = match offset {
            0x04 => u32::from(state.prescaler),
            0x08 => u32::from(state.reload),
            0x0c => 0,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(3)],
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 IWDG requires word accesses"));
        }
        let value = value as u32;
        let mut state = self.state.lock().expect("IWDG lock poisoned");
        match offset {
            0x00 => match value {
                IWDG_KEY_START => {
                    state.enabled = true;
                    state.started = at.ticks();
                }
                IWDG_KEY_REFRESH if state.enabled => {
                    state.started = at.ticks();
                    state.pending = false;
                }
                IWDG_KEY_UNLOCK => state.unlocked = true,
                _ => {}
            },
            0x04 if state.unlocked => state.prescaler = (value & IWDG_PR_MASK) as u8,
            0x08 if state.unlocked => state.reload = (value & IWDG_RLR_MASK) as u16,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(3)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("IWDG lock poisoned") = IwdgState {
            reload: 0xfff,
            ..IwdgState::default()
        };
        self.registers = [0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_reload_and_timeout_are_deterministic() {
        let (mut watchdog, handle) = Stm32Watchdog::new("iwdg");
        assert_eq!(
            watchdog
                .read(0x04, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            watchdog
                .read(0x08, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(IWDG_RLR_MASK)
        );
        watchdog
            .write(0x08, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            watchdog
                .read(0x08, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(IWDG_RLR_MASK)
        );
        watchdog
            .write(0x00, AccessWidth::Word, 0x5555, SimTime::ZERO)
            .unwrap();
        watchdog
            .write(0x08, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        watchdog
            .write(0x00, AccessWidth::Word, 0xcccc, SimTime::ZERO)
            .unwrap();
        assert!(!handle.take_reset(SimTime::from_ticks(15)));
        assert!(handle.take_reset(SimTime::from_ticks(16)));
        watchdog
            .write(0x00, AccessWidth::Word, 0xaaaa, SimTime::from_ticks(16))
            .unwrap();
        assert!(!handle.take_reset(SimTime::from_ticks(31)));
    }
}
