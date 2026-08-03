use super::*;

#[derive(Clone, Copy)]
struct PwmSlice {
    csr: u32,
    div: u32,
    ctr: u16,
    cc: u32,
    top: u16,
}

impl Default for PwmSlice {
    fn default() -> Self {
        Self {
            csr: 0,
            div: 0x10,
            ctr: 0,
            cc: 0,
            top: u16::MAX,
        }
    }
}

struct PwmState {
    slices: Vec<PwmSlice>,
    enable: u32,
    intr: u32,
    inte0: u32,
    intf0: u32,
    inte1: u32,
    intf1: u32,
    last_time: SimTime,
}

/// Named RP PWM register identifiers shared by RP2040 and RP2350.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpPwmRegister {
    /// Per-slice control/status register.
    Csr,
    /// Per-slice integer/fractional divider.
    Div,
    /// Per-slice counter.
    Ctr,
    /// Per-slice A/B compare values.
    Cc,
    /// Per-slice wrap value.
    Top,
    /// Global channel enable alias.
    En,
    /// Global raw interrupt status.
    Intr,
    /// Global IRQ0 interrupt enable.
    Inte0,
    /// Global IRQ0 interrupt force.
    Intf0,
    /// Global IRQ0 masked interrupt status.
    Ints0,
    /// RP2350 IRQ1 interrupt enable.
    Inte1,
    /// RP2350 IRQ1 interrupt force.
    Intf1,
    /// RP2350 IRQ1 masked interrupt status.
    Ints1,
}

impl RpPwmRegister {
    /// Returns a per-slice register offset.
    pub const fn slice_offset(self, slice: usize) -> Option<u64> {
        let local = match self {
            Self::Csr => 0x00,
            Self::Div => 0x04,
            Self::Ctr => 0x08,
            Self::Cc => 0x0c,
            Self::Top => 0x10,
            _ => return None,
        };
        Some(slice as u64 * 0x14 + local)
    }

    /// Returns a global register offset for a block with `slice_count` slices.
    pub const fn global_offset(self, slice_count: usize) -> Option<u64> {
        let base = slice_count as u64 * 0x14;
        let local = match self {
            Self::En => 0x00,
            Self::Intr => 0x04,
            Self::Inte0 => 0x08,
            Self::Intf0 => 0x0c,
            Self::Ints0 => 0x10,
            Self::Inte1 => 0x14,
            Self::Intf1 => 0x18,
            Self::Ints1 => 0x1c,
            _ => return None,
        };
        Some(base + local)
    }
}

/// Host-facing snapshot of RP PWM slices.
#[derive(Clone)]
pub struct PwmHandle {
    state: Arc<Mutex<PwmState>>,
}

impl PwmHandle {
    /// Returns the current high/low output state for channels A and B.
    pub fn outputs(&self, slice: usize) -> Option<[bool; 2]> {
        let state = self.state.lock().expect("PWM state lock poisoned");
        let slice = state.slices.get(slice)?;
        let mut outputs = [
            slice.ctr < (slice.cc & 0xffff) as u16,
            slice.ctr < (slice.cc >> 16) as u16,
        ];
        if slice.csr & (1 << 2) != 0 {
            outputs[0] = !outputs[0];
        }
        if slice.csr & (1 << 3) != 0 {
            outputs[1] = !outputs[1];
        }
        Some(outputs)
    }

    /// Returns the current counter for one slice.
    pub fn counter(&self, slice: usize) -> Option<u16> {
        self.state
            .lock()
            .expect("PWM state lock poisoned")
            .slices
            .get(slice)
            .map(|slice| slice.ctr)
    }

    /// Returns the IRQ0 pending bits after applying its interrupt mask.
    pub fn pending_interrupts(&self) -> u32 {
        self.pending_interrupts_for(0)
    }

    /// Returns pending bits for RP2350 IRQ0 or IRQ1.
    pub fn pending_interrupts_for(&self, irq: usize) -> u32 {
        let state = self.state.lock().expect("PWM state lock poisoned");
        match irq {
            0 => (state.intr & state.inte0) | state.intf0,
            1 => (state.intr & state.inte1) | state.intf1,
            _ => 0,
        }
    }
}

/// Deterministic RP2040/RP2350 PWM slice controller.
///
/// Functional time advances free-running counters between MMIO accesses;
/// fractional divider and exact phase-correct edge timing remain outside this
/// non-cycle-accurate slice. Register layout, masks, aliases, compare outputs,
/// wrap interrupts, and both RP2350 interrupt banks are modeled.
pub struct FunctionalPwm {
    name: String,
    state: Arc<Mutex<PwmState>>,
}

impl FunctionalPwm {
    const SLICE_STRIDE: u64 = 0x14;
    // CSR[7:6] are command bits (PH_ADV/PH_RET) and self-clear after a
    // write.  The remaining fields are persistent control bits.
    const CSR_MASK: u32 = 0xff;
    const CSR_PHASE_RET: u32 = 1 << 6;
    const CSR_PHASE_ADV: u32 = 1 << 7;
    const DIV_MASK: u32 = 0x0fff;
    const VALUE_MASK: u32 = 0xffff;

    /// Creates a PWM controller with `slice_count` independent slices.
    pub fn new(name: impl Into<String>, slice_count: usize) -> (Self, PwmHandle) {
        assert!(
            (1..=12).contains(&slice_count),
            "RP PWM slice count must be 1..=12"
        );
        let state = Arc::new(Mutex::new(PwmState {
            slices: vec![PwmSlice::default(); slice_count],
            enable: 0,
            intr: 0,
            inte0: 0,
            intf0: 0,
            inte1: 0,
            intf1: 0,
            last_time: SimTime::ZERO,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            PwmHandle { state },
        )
    }

    fn check_access(offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP PWM requires aligned word access"));
        }
        Ok(offset & 0x0fff)
    }

    fn decode(slice_count: usize, offset: u64) -> Result<(usize, RpPwmRegister), DeviceError> {
        let global_base = slice_count as u64 * Self::SLICE_STRIDE;
        if offset < global_base {
            let slice = usize::try_from(offset / Self::SLICE_STRIDE).expect("PWM slice fits");
            let register = match offset % Self::SLICE_STRIDE {
                0x00 => RpPwmRegister::Csr,
                0x04 => RpPwmRegister::Div,
                0x08 => RpPwmRegister::Ctr,
                0x0c => RpPwmRegister::Cc,
                0x10 => RpPwmRegister::Top,
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled RP PWM slice register at {offset:#x}"
                    )));
                }
            };
            return Ok((slice, register));
        }
        let register = match offset - global_base {
            0x00 => RpPwmRegister::En,
            0x04 => RpPwmRegister::Intr,
            0x08 => RpPwmRegister::Inte0,
            0x0c => RpPwmRegister::Intf0,
            0x10 => RpPwmRegister::Ints0,
            0x14 => RpPwmRegister::Inte1,
            0x18 => RpPwmRegister::Intf1,
            0x1c => RpPwmRegister::Ints1,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP PWM global register at {offset:#x}"
                )));
            }
        };
        Ok((0, register))
    }

    fn advance(state: &mut PwmState, at: SimTime) {
        let Some(delta) = at.checked_duration_since(state.last_time) else {
            return;
        };
        let delta = delta.ticks();
        if delta == 0 {
            return;
        }
        for (index, slice) in state.slices.iter_mut().enumerate() {
            if state.enable & (1 << index) == 0 || slice.csr & 1 == 0 {
                continue;
            }
            let period = u64::from(slice.top) + 1;
            let total = u64::from(slice.ctr) + delta;
            if total >= period {
                state.intr |= 1 << index;
            }
            slice.ctr = (total % period) as u16;
        }
        state.last_time = at;
    }

    fn adjust_phase(slice: &mut PwmSlice, command: u32) {
        let period = u32::from(slice.top) + 1;
        let counter = u32::from(slice.ctr);
        // PH_ADV and PH_RET are mutually exclusive commands in the hardware
        // programming model.  Treat an invalid request deterministically as
        // no adjustment rather than applying two contradictory operations.
        slice.ctr = match command & (Self::CSR_PHASE_ADV | Self::CSR_PHASE_RET) {
            Self::CSR_PHASE_ADV => ((counter + 1) % period) as u16,
            Self::CSR_PHASE_RET => ((counter + period - 1) % period) as u16,
            _ => slice.ctr,
        };
    }
}

impl Device for FunctionalPwm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let offset = Self::check_access(offset, width)?;
        let mut state = self.state.lock().expect("PWM state lock poisoned");
        Self::advance(&mut state, at);
        let (slice_index, register) = Self::decode(state.slices.len(), offset)?;
        let value = match register {
            RpPwmRegister::Csr => state.slices[slice_index].csr,
            RpPwmRegister::Div => state.slices[slice_index].div,
            RpPwmRegister::Ctr => u32::from(state.slices[slice_index].ctr),
            RpPwmRegister::Cc => state.slices[slice_index].cc,
            RpPwmRegister::Top => u32::from(state.slices[slice_index].top),
            RpPwmRegister::En => state.enable,
            RpPwmRegister::Intr => state.intr,
            RpPwmRegister::Inte0 => state.inte0,
            RpPwmRegister::Intf0 => state.intf0,
            RpPwmRegister::Ints0 => (state.intr & state.inte0) | state.intf0,
            RpPwmRegister::Inte1 => state.inte1,
            RpPwmRegister::Intf1 => state.intf1,
            RpPwmRegister::Ints1 => (state.intr & state.inte1) | state.intf1,
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
        let offset = Self::check_access(offset, width)?;
        let mut state = self.state.lock().expect("PWM state lock poisoned");
        Self::advance(&mut state, at);
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("PWM value fits");
        let (slice_index, register) = Self::decode(state.slices.len(), offset)?;
        match register {
            RpPwmRegister::Csr => {
                let phase_command = value & (Self::CSR_PHASE_ADV | Self::CSR_PHASE_RET);
                let slice = &mut state.slices[slice_index];
                Self::adjust_phase(slice, phase_command);
                // PH_ADV/PH_RET self-clear; all other CSR fields retain the
                // value written by firmware.
                slice.csr = value & (Self::CSR_MASK & !(Self::CSR_PHASE_ADV | Self::CSR_PHASE_RET));
                if slice.csr & 1 != 0 {
                    state.enable |= 1 << slice_index;
                } else {
                    state.enable &= !(1 << slice_index);
                }
            }
            RpPwmRegister::Div => state.slices[slice_index].div = value & Self::DIV_MASK,
            RpPwmRegister::Ctr => state.slices[slice_index].ctr = (value & Self::VALUE_MASK) as u16,
            RpPwmRegister::Cc => state.slices[slice_index].cc = value,
            RpPwmRegister::Top => state.slices[slice_index].top = (value & Self::VALUE_MASK) as u16,
            RpPwmRegister::En => {
                let mask = (1_u32 << state.slices.len()) - 1;
                state.enable = value & mask;
                let enable = state.enable;
                for (index, slice) in state.slices.iter_mut().enumerate() {
                    slice.csr = (slice.csr & !1) | ((enable >> index) & 1);
                }
            }
            RpPwmRegister::Intr => state.intr &= !value,
            RpPwmRegister::Inte0 => state.inte0 = value & ((1_u32 << state.slices.len()) - 1),
            RpPwmRegister::Intf0 => state.intf0 = value & ((1_u32 << state.slices.len()) - 1),
            RpPwmRegister::Ints0 => return Err(DeviceError::new("RP PWM INTS0 is read-only")),
            RpPwmRegister::Inte1 => state.inte1 = value & ((1_u32 << state.slices.len()) - 1),
            RpPwmRegister::Intf1 => state.intf1 = value & ((1_u32 << state.slices.len()) - 1),
            RpPwmRegister::Ints1 => return Err(DeviceError::new("RP PWM INTS1 is read-only")),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("PWM state lock poisoned");
        for slice in &mut state.slices {
            *slice = PwmSlice::default();
        }
        state.enable = 0;
        state.intr = 0;
        state.inte0 = 0;
        state.intf0 = 0;
        state.inte1 = 0;
        state.intf1 = 0;
        state.last_time = SimTime::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_register_offsets_follow_target_slice_count() {
        assert_eq!(RpPwmRegister::En.global_offset(8), Some(0xa0));
        assert_eq!(RpPwmRegister::Intr.global_offset(12), Some(0xf4));
        assert_eq!(RpPwmRegister::Ints1.global_offset(12), Some(0x10c));
        assert_eq!(RpPwmRegister::Cc.slice_offset(3), Some(0x48));
    }

    #[test]
    fn global_alias_and_interrupt_banks_are_functional() {
        let (mut pwm, handle) = FunctionalPwm::new("rp2350.pwm", 12);
        pwm.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(pwm.read(0xf0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        pwm.write(0xf8, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0xfc, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_interrupts_for(0), 1);
        pwm.write(0x104, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0x108, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_interrupts_for(1), 1);
        assert!(
            pwm.write(0x100, AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
    }
}
