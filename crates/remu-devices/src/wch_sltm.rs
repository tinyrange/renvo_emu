use super::*;

/// Register offsets for the CH32V006 streamlined TIM3 block.
#[repr(u64)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WchSltmRegister {
    Control = 0x00,
    DmaIntEnable = 0x04,
    Counter = 0x08,
    AutoReload = 0x0c,
    Compare1 = 0x10,
    Compare2 = 0x14,
    Compare3 = 0x18,
    Compare4 = 0x1c,
}

impl WchSltmRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Control,
            0x04 => Self::DmaIntEnable,
            0x08 => Self::Counter,
            0x0c => Self::AutoReload,
            0x10 => Self::Compare1,
            0x14 => Self::Compare2,
            0x18 => Self::Compare3,
            0x1c => Self::Compare4,
            _ => return None,
        })
    }

    fn compare_index(self) -> Option<usize> {
        Some(match self {
            Self::Compare1 => 0,
            Self::Compare2 => 1,
            Self::Compare3 => 2,
            Self::Compare4 => 3,
            _ => return None,
        })
    }
}

/// Events reported by [`WchSltmHandle::poll`].
pub mod wch_sltm_events {
    /// The counter wrapped through its auto-reload value.
    pub const UPDATE: u8 = 1 << 0;
    /// Compare channel one reached its compare value.
    pub const COMPARE1: u8 = 1 << 1;
    /// Compare channel two reached its compare value.
    pub const COMPARE2: u8 = 1 << 2;
    /// Compare channel three reached its compare value.
    pub const COMPARE3: u8 = 1 << 3;
    /// Compare channel four reached its compare value.
    pub const COMPARE4: u8 = 1 << 4;
}

/// Scheduler-facing handle for the CH32V006 streamlined TIM3.
#[derive(Clone)]
pub struct WchSltmHandle {
    state: Rc<RefCell<WchSltmState>>,
}

impl WchSltmHandle {
    /// Advances the timer and returns events crossed since the previous poll.
    pub fn poll(&self, now: SimTime) -> u8 {
        self.state.borrow_mut().update(now)
    }
}

struct WchSltmState {
    control: u16,
    dma_int_enable: u16,
    counter: u16,
    auto_reload: u16,
    compare: [u16; 4],
    epoch: u64,
    start_counter: u16,
    last_elapsed: u64,
}

impl WchSltmState {
    const CEN: u16 = 1 << 0;
    const DIR: u16 = 1 << 4;
    const SMS_MASK: u16 = 0x0700;

    fn reset() -> Self {
        Self {
            control: 0,
            dma_int_enable: 0,
            counter: 0,
            auto_reload: u16::MAX,
            compare: [0; 4],
            epoch: 0,
            start_counter: 0,
            last_elapsed: 0,
        }
    }

    fn period(&self) -> u64 {
        u64::from(self.auto_reload) + 1
    }

    fn running(&self) -> bool {
        self.control & Self::CEN != 0 && self.control & Self::SMS_MASK == 0
    }

    fn counter_at(&self, elapsed: u64) -> u16 {
        let period = self.period();
        let phase = elapsed % period;
        let start = u64::from(self.start_counter) % period;
        let value = if self.control & Self::DIR == 0 {
            (start + phase) % period
        } else {
            (start + period - phase) % period
        };
        u16::try_from(value).expect("streamlined timer counter fits u16")
    }

    fn first_hit_after(start: u64, period: u64, residue: u64) -> u64 {
        let first = if start < residue {
            residue
        } else {
            let distance = start - residue + 1;
            residue + distance.div_ceil(period) * period
        };
        first
    }

    fn has_hit(&self, target: u16, end_elapsed: u64) -> bool {
        if end_elapsed <= self.last_elapsed {
            return false;
        }
        let period = self.period();
        let start = u64::from(self.start_counter) % period;
        let target = u64::from(target) % period;
        let residue = if self.control & Self::DIR == 0 {
            (target + period - start) % period
        } else {
            (start + period - target) % period
        };
        Self::first_hit_after(self.last_elapsed, period, residue) <= end_elapsed
    }

    fn update(&mut self, now: SimTime) -> u8 {
        if !self.running() {
            self.epoch = now.ticks();
            self.last_elapsed = 0;
            return 0;
        }
        let elapsed = now.ticks().saturating_sub(self.epoch);
        if elapsed <= self.last_elapsed {
            self.counter = self.counter_at(elapsed);
            return 0;
        }
        let period = self.period();
        let wraps = elapsed / period > self.last_elapsed / period;
        let mut events = 0;
        if wraps && self.control & (1 << 1) == 0 {
            events |= wch_sltm_events::UPDATE;
        }
        for (index, compare) in self.compare.into_iter().enumerate() {
            if index == 2 && self.dma_int_enable & (1 << 11) == 0 {
                continue;
            }
            if index == 3 && self.dma_int_enable & (1 << 12) == 0 {
                continue;
            }
            if self.has_hit(compare, elapsed) {
                events |= 1 << (index + 1);
            }
        }
        self.counter = self.counter_at(elapsed);
        self.last_elapsed = elapsed;
        events
    }

    fn restart(&mut self, now: SimTime) {
        self.epoch = now.ticks();
        self.start_counter = self.counter;
        self.last_elapsed = 0;
    }
}

/// Functional CH32V006 streamlined TIM3 counter and compare block.
pub struct WchSltm {
    name: String,
    state: Rc<RefCell<WchSltmState>>,
}

impl WchSltm {
    /// Creates a reset streamlined timer and scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, WchSltmHandle) {
        let state = Rc::new(RefCell::new(WchSltmState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchSltmHandle { state },
        )
    }

    fn require_register_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH streamlined TIM requires halfword or word access at a register boundary",
            ));
        }
        Ok(())
    }
}

impl Device for WchSltm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        Self::require_register_access(offset, width)?;
        let register = WchSltmRegister::from_offset(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled WCH TIM3 read at {offset:#x}")))?;
        let mut state = self.state.borrow_mut();
        state.update(at);
        let value = match register {
            WchSltmRegister::Control => state.control,
            WchSltmRegister::DmaIntEnable => state.dma_int_enable,
            WchSltmRegister::Counter => state.counter,
            WchSltmRegister::AutoReload => state.auto_reload,
            register => {
                state.compare[register
                    .compare_index()
                    .expect("compare register has an index")]
            }
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
        Self::require_register_access(offset, width)?;
        let register = WchSltmRegister::from_offset(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled WCH TIM3 write at {offset:#x}")))?;
        let value = u16::try_from(value & u64::from(u16::MAX))
            .expect("masked WCH TIM3 register value fits u16");
        let mut state = self.state.borrow_mut();
        state.update(at);
        match register {
            WchSltmRegister::Control => {
                let was_running = state.running();
                state.control = value;
                if !was_running && state.running() {
                    state.restart(at);
                }
            }
            WchSltmRegister::DmaIntEnable => state.dma_int_enable = value,
            WchSltmRegister::Counter => {
                state.counter = value;
                state.restart(at);
            }
            WchSltmRegister::AutoReload => {
                state.auto_reload = value;
                state.restart(at);
            }
            register => {
                state.compare[register
                    .compare_index()
                    .expect("compare register has an index")] = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchSltmState::reset();
    }
}
