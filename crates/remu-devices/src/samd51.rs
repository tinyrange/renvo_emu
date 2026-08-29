use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const CTRLA_ENABLE: u32 = 1 << 1;
const INTERRUPT_MC0: u8 = 1 << 4;

#[derive(Default)]
struct TcState {
    ctrla: u32,
    interrupt_enable: u8,
    interrupt_flags: u8,
    started: u64,
    count: u16,
    compare: [u16; 2],
    matched: bool,
}

/// Machine-facing handle for a SAM D51 COUNT16 timer instance.
#[derive(Clone)]
pub struct Samd51TcHandle(Arc<Mutex<TcState>>);

impl Samd51TcHandle {
    /// Advances TC0 in deterministic abstract ticks and returns its IRQ level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("SAMD51 TC lock poisoned");
        let elapsed = now.ticks().saturating_sub(state.started);
        if state.ctrla & CTRLA_ENABLE != 0
            && !state.matched
            && state.compare[0] != 0
            && elapsed >= u64::from(state.compare[0].wrapping_sub(state.count))
        {
            state.interrupt_flags |= INTERRUPT_MC0;
            state.matched = true;
        }
        state.interrupt_enable & state.interrupt_flags != 0
    }
}

/// SAM D51 TC COUNT16 register slice using the native D5x/E5x offsets.
pub struct Samd51Tc {
    name: String,
    state: Arc<Mutex<TcState>>,
    control_b: u8,
    event_control: u16,
    wave: u8,
    driver_control: u8,
    debug_control: u8,
}

impl Samd51Tc {
    /// Creates a reset TC instance and its machine-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd51TcHandle) {
        let state = Arc::new(Mutex::new(TcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                control_b: 0,
                event_control: 0,
                wave: 0,
                driver_control: 0,
                debug_control: 0,
            },
            Samd51TcHandle(state),
        )
    }

    fn counter(state: &TcState, at: SimTime) -> u16 {
        if state.ctrla & CTRLA_ENABLE == 0 {
            state.count
        } else {
            state
                .count
                .wrapping_add(at.ticks().saturating_sub(state.started) as u16)
        }
    }
}

impl Device for Samd51Tc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("SAMD51 TC lock poisoned");
        let value = match (offset, width) {
            (0x00, AccessWidth::Word) => u64::from(state.ctrla),
            (0x04 | 0x05, AccessWidth::Byte) => u64::from(self.control_b),
            (0x06, AccessWidth::HalfWord) => u64::from(self.event_control),
            (0x08 | 0x09, AccessWidth::Byte) => u64::from(state.interrupt_enable),
            (0x0a, AccessWidth::Byte) => u64::from(state.interrupt_flags),
            (0x0b, AccessWidth::Byte) => 1 << 3,
            (0x0c, AccessWidth::Byte) => u64::from(self.wave),
            (0x0d, AccessWidth::Byte) => u64::from(self.driver_control),
            (0x0f, AccessWidth::Byte) => u64::from(self.debug_control),
            (0x10, AccessWidth::Word) => 0,
            (0x14, AccessWidth::HalfWord) => u64::from(Self::counter(&state, at)),
            (0x1c, AccessWidth::HalfWord) => u64::from(state.compare[0]),
            (0x1e, AccessWidth::HalfWord) => u64::from(state.compare[1]),
            _ => {
                return Err(DeviceError::new(format!(
                    "invalid SAMD51 TC read at offset {offset:#x} with {width:?}"
                )));
            }
        };
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("SAMD51 TC lock poisoned");
        match (offset, width) {
            (0x00, AccessWidth::Word) => {
                let was_enabled = state.ctrla & CTRLA_ENABLE != 0;
                let current = Self::counter(&state, at);
                state.ctrla = value as u32 & 0x00ff_0f43;
                if value & 1 != 0 {
                    *state = TcState::default();
                    return Ok(());
                }
                let enabled = state.ctrla & CTRLA_ENABLE != 0;
                if enabled && !was_enabled {
                    state.started = at.ticks();
                    state.matched = false;
                } else if !enabled && was_enabled {
                    state.count = current;
                }
            }
            (0x04, AccessWidth::Byte) => self.control_b &= !(value as u8 & 0xc3),
            (0x05, AccessWidth::Byte) => self.control_b |= value as u8 & 0xc3,
            (0x06, AccessWidth::HalfWord) => self.event_control = value as u16 & 0x7f3f,
            (0x08, AccessWidth::Byte) => state.interrupt_enable &= !(value as u8 & 0x33),
            (0x09, AccessWidth::Byte) => state.interrupt_enable |= value as u8 & 0x33,
            (0x0a, AccessWidth::Byte) => state.interrupt_flags &= !(value as u8 & 0x33),
            (0x0b, AccessWidth::Byte) => {}
            (0x0c, AccessWidth::Byte) => self.wave = value as u8 & 3,
            (0x0d, AccessWidth::Byte) => self.driver_control = value as u8 & 3,
            (0x0f, AccessWidth::Byte) => self.debug_control = value as u8 & 1,
            (0x10, AccessWidth::Word) => {}
            (0x14, AccessWidth::HalfWord) => {
                state.count = value as u16;
                state.started = at.ticks();
                state.matched = false;
            }
            (0x1c, AccessWidth::HalfWord) => {
                state.compare[0] = value as u16;
                state.matched = false;
            }
            (0x1e, AccessWidth::HalfWord) => state.compare[1] = value as u16,
            _ => {
                return Err(DeviceError::new(format!(
                    "invalid SAMD51 TC write at offset {offset:#x} with {width:?}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("SAMD51 TC lock poisoned") = TcState::default();
        self.control_b = 0;
        self.event_control = 0;
        self.wave = 0;
        self.driver_control = 0;
        self.debug_control = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_count16_offsets_raise_match_zero() {
        let (mut timer, handle) = Samd51Tc::new("tc0");
        timer
            .write(0x1c, AccessWidth::HalfWord, 5, SimTime::ZERO)
            .expect("CC0 write");
        timer
            .write(0x09, AccessWidth::Byte, INTERRUPT_MC0.into(), SimTime::ZERO)
            .expect("INTENSET write");
        timer
            .write(0x00, AccessWidth::Word, CTRLA_ENABLE.into(), SimTime::ZERO)
            .expect("CTRLA write");
        assert!(!handle.poll(SimTime::from_ticks(4)));
        assert!(handle.poll(SimTime::from_ticks(5)));
        assert_eq!(
            timer
                .read(0x0a, AccessWidth::Byte, SimTime::from_ticks(5))
                .expect("INTFLAG read"),
            u64::from(INTERRUPT_MC0)
        );
    }

    #[test]
    fn disabling_count16_preserves_the_elapsed_counter() {
        let (mut timer, _) = Samd51Tc::new("tc0");
        timer
            .write(0x00, AccessWidth::Word, CTRLA_ENABLE.into(), SimTime::ZERO)
            .expect("CTRLA enable");
        timer
            .write(0x00, AccessWidth::Word, 0, SimTime::from_ticks(7))
            .expect("CTRLA disable");
        assert_eq!(
            timer
                .read(0x14, AccessWidth::HalfWord, SimTime::from_ticks(20))
                .expect("COUNT read"),
            7
        );
    }
}
