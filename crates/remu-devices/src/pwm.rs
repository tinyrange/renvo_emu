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
    inte: u32,
    intf: u32,
    last_time: SimTime,
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
            u32::from(slice.ctr < (slice.cc & 0xffff) as u16) != 0,
            u32::from(slice.ctr < (slice.cc >> 16) as u16) != 0,
        ];
        if slice.csr & (1 << 1) != 0 {
            outputs[0] = !outputs[0];
        }
        if slice.csr & (1 << 2) != 0 {
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

    /// Returns the enabled interrupt bits after applying the interrupt mask.
    pub fn pending_interrupts(&self) -> u32 {
        let state = self.state.lock().expect("PWM state lock poisoned");
        (state.intr & state.inte) | state.intf
    }
}

/// Deterministic RP2040/RP2350 PWM slice controller.
///
/// Each slice exposes the RP PWM `CSR`, `DIV`, `CTR`, `CC`, and `TOP` registers. Functional time
/// advances the free-running counters between MMIO accesses; compare outputs can be inspected via
/// [`PwmHandle::outputs`]. The model intentionally omits pin muxing, fractional divider timing,
/// phase-correct edge details, and DMA pacing while retaining useful duty-cycle behavior.
pub struct FunctionalPwm {
    name: String,
    state: Arc<Mutex<PwmState>>,
}

impl FunctionalPwm {
    const SLICE_STRIDE: u64 = 0x14;
    const EN: u64 = 0xa0;
    const INTR: u64 = 0xa4;
    const INTE: u64 = 0xa8;
    const INTF: u64 = 0xac;
    const INTS: u64 = 0xb0;

    /// Creates a PWM controller with `slice_count` independent slices.
    pub fn new(name: impl Into<String>, slice_count: usize) -> (Self, PwmHandle) {
        let state = Arc::new(Mutex::new(PwmState {
            slices: vec![PwmSlice::default(); slice_count],
            enable: 0,
            intr: 0,
            inte: 0,
            intf: 0,
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
            return Err(DeviceError::new("PWM requires aligned word access"));
        }
        Ok(offset & 0x0fff)
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

    fn read_register(state: &mut PwmState, offset: u64, at: SimTime) -> u32 {
        Self::advance(state, at);
        if offset < Self::EN {
            let index = usize::try_from(offset / Self::SLICE_STRIDE).expect("PWM slice fits");
            let register = offset % Self::SLICE_STRIDE;
            return state.slices.get(index).map_or(0, |slice| match register {
                0x00 => slice.csr,
                0x04 => slice.div,
                0x08 => u32::from(slice.ctr),
                0x0c => slice.cc,
                0x10 => u32::from(slice.top),
                _ => 0,
            });
        }
        match offset {
            Self::EN => state.enable,
            Self::INTR => state.intr,
            Self::INTE => state.inte,
            Self::INTF => state.intf,
            Self::INTS => (state.intr & state.inte) | state.intf,
            _ => 0,
        }
    }
}

impl Device for FunctionalPwm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let offset = Self::check_access(offset, width)?;
        Ok(u64::from(Self::read_register(
            &mut self.state.lock().expect("PWM state lock poisoned"),
            offset,
            at,
        )))
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
        if offset < Self::EN {
            let index = usize::try_from(offset / Self::SLICE_STRIDE).expect("PWM slice fits");
            let register = offset % Self::SLICE_STRIDE;
            let Some(slice) = state.slices.get_mut(index) else {
                return Err(DeviceError::new("PWM slice outside modeled range"));
            };
            match register {
                0x00 => slice.csr = value & 0x3f,
                0x04 => slice.div = value & 0x0fff,
                0x08 => slice.ctr = value as u16,
                0x0c => slice.cc = value,
                0x10 => slice.top = value as u16,
                _ => return Err(DeviceError::new("unmodeled PWM slice register")),
            }
            return Ok(());
        }
        match offset {
            Self::EN => state.enable = value,
            Self::INTR => state.intr &= !value,
            Self::INTE => state.inte = value,
            Self::INTF => state.intf = value,
            Self::INTS => {}
            _ => return Err(DeviceError::new("unmodeled PWM register")),
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
        state.inte = 0;
        state.intf = 0;
        state.last_time = SimTime::ZERO;
    }
}
