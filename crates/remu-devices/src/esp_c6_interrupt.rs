use super::*;

#[derive(Clone)]
/// Host-facing input and delivery view of both ESP32-C6 PLIC contexts.
pub struct EspC6PlicHandle {
    state: Rc<RefCell<EspC6PlicState>>,
}

impl EspC6PlicHandle {
    /// Drives one of the 32 CPU-local interrupt inputs.
    pub fn set_line(&self, line: u8, asserted: bool) {
        if line >= 32 {
            return;
        }
        let bit = 1_u32 << line;
        let mut state = self.state.borrow_mut();
        let rising = asserted && state.levels & bit == 0;
        if asserted {
            state.levels |= bit;
        } else {
            state.levels &= !bit;
        }
        for context in &mut state.contexts {
            if context.edge & bit != 0 {
                if rising {
                    context.pending |= bit;
                }
            } else if asserted {
                context.pending |= bit;
            } else {
                context.pending &= !bit;
            }
        }
    }

    /// Returns the machine/user local lines that pass enable, pending,
    /// priority, and threshold selection.
    pub fn deliverable(&self, user: bool) -> u32 {
        self.state.borrow().deliverable(usize::from(user))
    }
}

#[derive(Clone, Default)]
struct EspC6PlicContext {
    enable: u32,
    edge: u32,
    pending: u32,
    priorities: [u8; 32],
    threshold: u8,
    active: u32,
    config: u32,
}

struct EspC6PlicState {
    contexts: [EspC6PlicContext; 2],
    levels: u32,
}

impl EspC6PlicState {
    fn deliverable(&self, context: usize) -> u32 {
        let context = &self.contexts[context];
        let candidates = context.enable & context.pending & !context.active;
        (0..32).fold(0, |mask, line| {
            let priority = context.priorities[line];
            if candidates & (1 << line) != 0 && priority >= context.threshold {
                mask | (1 << line)
            } else {
                mask
            }
        })
    }

    fn claim(&mut self, context: usize) -> u32 {
        let deliverable = self.deliverable(context);
        let selected = (0..32)
            .filter(|line| deliverable & (1 << line) != 0)
            .max_by_key(|line| (self.contexts[context].priorities[*line], 31 - *line));
        let Some(line) = selected else {
            return u32::MAX;
        };
        let bit = 1_u32 << line;
        let target = &mut self.contexts[context];
        target.active |= bit;
        if target.edge & bit != 0 {
            target.pending &= !bit;
        }
        line as u32
    }
}

/// One machine or user page of the ESP32-C6 CPU-local PLIC.
pub struct EspC6Plic {
    name: String,
    context: usize,
    state: Rc<RefCell<EspC6PlicState>>,
}

impl EspC6Plic {
    /// Creates the machine and user pages backed by the same 32 input lines.
    pub fn new_pair(
        machine_name: impl Into<String>,
        user_name: impl Into<String>,
    ) -> (Self, Self, EspC6PlicHandle) {
        let state = Rc::new(RefCell::new(EspC6PlicState {
            contexts: [EspC6PlicContext::default(), EspC6PlicContext::default()],
            levels: 0,
        }));
        (
            Self {
                name: machine_name.into(),
                context: 0,
                state: state.clone(),
            },
            Self {
                name: user_name.into(),
                context: 1,
                state: state.clone(),
            },
            EspC6PlicHandle { state },
        )
    }

    fn check(offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x400 {
            return Err(DeviceError::new(
                "ESP32-C6 PLIC requires an aligned word access",
            ));
        }
        Ok(offset as usize)
    }
}

impl Device for EspC6Plic {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let offset = Self::check(offset, width)?;
        let mut state = self.state.borrow_mut();
        let value = match offset {
            0x00 => state.contexts[self.context].enable,
            0x04 => state.contexts[self.context].edge,
            0x08 => 0,
            0x0c => state.contexts[self.context].pending,
            0x10..=0x8c => u32::from(state.contexts[self.context].priorities[(offset - 0x10) / 4]),
            0x90 => u32::from(state.contexts[self.context].threshold),
            0x94 => state.claim(self.context),
            0x3fc => state.contexts[self.context].config,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
                )));
            }
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
        let offset = Self::check(offset, width)?;
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        let levels = state.levels;
        let context = &mut state.contexts[self.context];
        match offset {
            0x00 => context.enable = value,
            0x04 => {
                context.edge = value;
                context.pending = (context.pending & value) | (levels & !value);
            }
            0x08 => context.pending &= !value,
            0x0c => {}
            0x10..=0x8c => context.priorities[(offset - 0x10) / 4] = (value & 0xf) as u8,
            0x90 => context.threshold = (value & 0xff) as u8,
            0x94 => {
                if value < 32 {
                    context.active &= !(1 << value);
                    if context.edge & (1 << value) == 0 && levels & (1 << value) != 0 {
                        context.pending |= 1 << value;
                    }
                }
            }
            0x3fc => context.config = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.contexts[self.context] = EspC6PlicContext::default();
        if self.context == 0 {
            state.levels = 0;
        }
    }
}

#[derive(Clone, Copy)]
struct ClintTimer {
    control: u32,
    base: u64,
    epoch: SimTime,
    compare: u64,
    software: bool,
}

impl ClintTimer {
    fn new(user: bool) -> Self {
        Self {
            control: 0,
            base: 0,
            epoch: SimTime::ZERO,
            compare: 0,
            software: user,
        }
    }

    fn value(self, now: SimTime) -> u64 {
        if self.control & 1 == 0 {
            self.base
        } else {
            self.base
                .wrapping_add(now.ticks().saturating_sub(self.epoch.ticks()))
        }
    }

    fn materialize(&mut self, now: SimTime) {
        self.base = self.value(now);
        self.epoch = now;
    }

    fn timer_pending(self, now: SimTime) -> bool {
        self.control & 3 == 3 && self.value(now) >= self.compare
    }
}

struct EspC6ClintState {
    timers: [ClintTimer; 2],
}

#[derive(Clone)]
/// Host-facing machine/user software and timer interrupt view.
pub struct EspC6ClintHandle {
    state: Rc<RefCell<EspC6ClintState>>,
}

impl EspC6ClintHandle {
    /// Returns machine software/timer and user software/timer requests.
    pub fn pending(&self, now: SimTime) -> [bool; 4] {
        let state = self.state.borrow();
        [
            state.timers[0].software,
            state.timers[0].timer_pending(now),
            state.timers[1].software,
            state.timers[1].timer_pending(now),
        ]
    }
}

/// ESP32-C6 machine and user CLINT pages.
pub struct EspC6Clint {
    name: String,
    state: Rc<RefCell<EspC6ClintState>>,
}

impl EspC6Clint {
    /// Creates the combined machine/user CLINT page and scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6ClintHandle) {
        let state = Rc::new(RefCell::new(EspC6ClintState {
            timers: [ClintTimer::new(false), ClintTimer::new(true)],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6ClintHandle { state },
        )
    }

    fn select(offset: u64, width: AccessWidth) -> Result<(usize, usize), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x800 {
            return Err(DeviceError::new(
                "ESP32-C6 CLINT requires an aligned word access",
            ));
        }
        Ok((usize::from(offset >= 0x400), (offset as usize) & 0x3ff))
    }
}

impl Device for EspC6Clint {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let (context, offset) = Self::select(offset, width)?;
        let state = self.state.borrow();
        let timer = state.timers[context];
        let counter = timer.value(at);
        let pending = timer.timer_pending(at);
        let value = match offset {
            0x00 => u32::from(timer.software),
            0x04 => (timer.control & 0x3b) | (u32::from(pending) << 2),
            0x08 => counter as u32,
            0x0c => (counter >> 32) as u32,
            0x10 => timer.compare as u32,
            0x14 => (timer.compare >> 32) as u32,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
                )));
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
        let (context, offset) = Self::select(offset, width)?;
        let mut state = self.state.borrow_mut();
        let timer = &mut state.timers[context];
        let value = value as u32;
        match offset {
            0x00 => timer.software = value & 1 != 0,
            0x04 => {
                timer.materialize(at);
                timer.control = value & 0x3b;
            }
            0x08 => {
                timer.materialize(at);
                timer.base = (timer.base & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            0x0c => {}
            0x10 => timer.compare = (timer.compare & 0xffff_ffff_0000_0000) | u64::from(value),
            0x14 => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().timers = [ClintTimer::new(false), ClintTimer::new(true)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plic_filters_claims_and_reasserts_level_requests() {
        let (mut machine, _, handle) = EspC6Plic::new_pair("m", "u");
        machine
            .write(0, AccessWidth::Word, 1 << 5, SimTime::ZERO)
            .unwrap();
        machine
            .write(0x24, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        machine
            .write(0x90, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        handle.set_line(5, true);
        assert_eq!(handle.deliverable(false), 1 << 5);
        assert_eq!(
            machine
                .read(0x94, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            5
        );
        assert_eq!(handle.deliverable(false), 0);
        machine
            .write(0x94, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.deliverable(false), 1 << 5);
        handle.set_line(5, false);
        assert_eq!(handle.deliverable(false), 0);
    }

    #[test]
    fn plic_accepts_an_interrupt_at_the_programmed_threshold_level() {
        let (mut machine, _, handle) = EspC6Plic::new_pair("m", "u");
        machine
            .write(0, AccessWidth::Word, 1 << 8, SimTime::ZERO)
            .unwrap();
        machine
            .write(0x30, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        machine
            .write(0x90, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.set_line(8, true);
        assert_eq!(handle.deliverable(false), 1 << 8);
    }

    #[test]
    fn plic_edge_request_latches_until_claim() {
        let (mut machine, _, handle) = EspC6Plic::new_pair("m", "u");
        machine
            .write(0, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        machine
            .write(4, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        machine
            .write(0x18, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.set_line(2, true);
        handle.set_line(2, false);
        assert_eq!(handle.deliverable(false), 1 << 2);
        assert_eq!(
            machine
                .read(0x94, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            2
        );
        assert_eq!(handle.deliverable(false), 0);
    }

    #[test]
    fn clint_delivers_independent_machine_and_user_timers() {
        let (mut clint, handle) = EspC6Clint::new("clint");
        clint
            .write(0x10, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        clint
            .write(0x04, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        clint
            .write(0x400, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            handle.pending(SimTime::from_ticks(4)),
            [false, false, true, false]
        );
        assert_eq!(
            handle.pending(SimTime::from_ticks(5)),
            [false, true, true, false]
        );
        assert_eq!(
            clint
                .read(0x04, AccessWidth::Word, SimTime::from_ticks(5))
                .unwrap()
                & 4,
            4
        );
    }
}
