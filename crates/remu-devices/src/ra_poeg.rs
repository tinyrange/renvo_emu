//! RA4M1 Port Output Enable for GPT (POEG) subset.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const GROUP_STRIDE: u64 = 0x100;
const STATUS_MASK: u32 = 0x0000_000f;
const CONFIG_MASK: u32 = (0x0f << 27) | (0x7 << 4);
const PIDF: u32 = 1 << 0;
const IOCF: u32 = 1 << 1;
const OSTPF: u32 = 1 << 2;
const SSF: u32 = 1 << 3;
const PIDE: u32 = 1 << 4;
const IOCE: u32 = 1 << 5;
const OSTPE: u32 = 1 << 6;
const ST: u32 = 1 << 16;

#[derive(Default)]
struct PoegState {
    groups: [u32; 4],
}

/// Host-facing POEG state for four GPT output groups.
#[derive(Clone)]
pub struct RaPoegHandle(Arc<Mutex<PoegState>>);

impl RaPoegHandle {
    fn trigger(&self, group: usize, flag: u32, enabled: u32) {
        if let Some(register) = self
            .0
            .lock()
            .expect("RA POEG lock poisoned")
            .groups
            .get_mut(group)
        {
            if *register & enabled != 0 {
                *register |= flag;
            }
        }
    }

    /// Injects a filtered GTETRG input level and optional output-disable flag.
    pub fn trigger_pin(&self, group: u8, high: bool) {
        let group = usize::from(group).min(3);
        let mut state = self.0.lock().expect("RA POEG lock poisoned");
        let register = &mut state.groups[group];
        if high {
            *register |= ST;
        } else {
            *register &= !ST;
        }
        if *register & PIDE != 0 {
            *register |= PIDF;
        }
    }

    /// Injects a GPT output-disable request.
    pub fn trigger_gpt(&self, group: u8) {
        self.trigger(usize::from(group).min(3), IOCF, IOCE);
    }

    /// Injects a main-oscillator-stop request.
    pub fn trigger_oscillation_stop(&self, group: u8) {
        self.trigger(usize::from(group).min(3), OSTPF, OSTPE);
    }

    /// Requests a software output stop for one group.
    pub fn software_stop(&self, group: u8) {
        let group = usize::from(group).min(3);
        self.0.lock().expect("RA POEG lock poisoned").groups[group] |= SSF;
    }

    /// Returns whether any configured POEG request has disabled the group.
    pub fn output_disabled(&self, group: u8) -> bool {
        self.0
            .lock()
            .expect("RA POEG lock poisoned")
            .groups
            .get(usize::from(group).min(3))
            .is_some_and(|register| register & STATUS_MASK != 0)
    }
}

/// Functional RA4M1 POEG group register block.
pub struct RaPoeg {
    name: String,
    state: Arc<Mutex<PoegState>>,
}

impl RaPoeg {
    /// Creates reset-state POEG groups A-D.
    pub fn new(name: impl Into<String>) -> (Self, RaPoegHandle) {
        let state = Arc::new(Mutex::new(PoegState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaPoegHandle(state),
        )
    }
}

impl Device for RaPoeg {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RA POEG requires aligned word access"));
        }
        let group = usize::try_from(offset / GROUP_STRIDE).unwrap_or(usize::MAX);
        if group >= 4 || offset % GROUP_STRIDE != 0 {
            return Err(DeviceError::new(format!(
                "unmodeled RA POEG read at {offset:#x}"
            )));
        }
        Ok(u64::from(
            self.state.lock().expect("RA POEG lock poisoned").groups[group],
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RA POEG requires aligned word access"));
        }
        let group = usize::try_from(offset / GROUP_STRIDE).unwrap_or(usize::MAX);
        if group >= 4 || offset % GROUP_STRIDE != 0 {
            return Err(DeviceError::new(format!(
                "unmodeled RA POEG write at {offset:#x}"
            )));
        }
        let mut state = self.state.lock().expect("RA POEG lock poisoned");
        let register = &mut state.groups[group];
        let value = value as u32;
        *register = (*register & ST) | (value & CONFIG_MASK);
        *register &= !(value & STATUS_MASK);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("RA POEG lock poisoned")
            .groups
            .fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_and_gpt_triggers_latch_and_clear_flags() {
        let (mut poeg, handle) = RaPoeg::new("poeg");
        poeg.write(0, AccessWidth::Word, u64::from(PIDE | IOCE), SimTime::ZERO)
            .unwrap();
        handle.trigger_pin(0, true);
        handle.trigger_gpt(0);
        assert!(handle.output_disabled(0));
        assert_eq!(
            poeg.read(0, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & (PIDF | IOCF | ST),
            PIDF | IOCF | ST
        );
        poeg.write(0, AccessWidth::Word, u64::from(PIDF | IOCF), SimTime::ZERO)
            .unwrap();
        assert!(!handle.output_disabled(0));
    }

    #[test]
    fn software_stop_is_host_visible_per_group() {
        let (mut poeg, handle) = RaPoeg::new("poeg");
        handle.software_stop(2);
        assert!(handle.output_disabled(2));
        assert_eq!(
            poeg.read(0x200, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & SSF,
            SSF
        );
    }
}
