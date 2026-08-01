use super::*;

/// Shared scheduler-facing state for a WCH general-purpose timer.
#[derive(Clone)]
pub struct WchTimerHandle {
    state: Rc<RefCell<WchTimerState>>,
}

impl WchTimerHandle {
    /// Advances the functional timer and reports its masked update interrupt.
    pub fn pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.update(now);
        state.intfr & state.dmaintenr & WchTimerState::UPDATE != 0
    }
}

struct WchTimerState {
    ctlr1: u16,
    ctlr2: u16,
    smcfgr: u16,
    dmaintenr: u16,
    intfr: u16,
    chctlr1: u16,
    chctlr2: u16,
    ccer: u16,
    cnt: u16,
    psc: u16,
    atrlr: u16,
    rptcr: u16,
    chcv: [u16; 4],
    bdtr: u16,
    dmacfgr: u16,
    dmaadr: u16,
    epoch: u64,
}

impl WchTimerState {
    const CEN: u16 = 1;
    const UPDATE: u16 = 1;

    fn reset() -> Self {
        Self {
            ctlr1: 0,
            ctlr2: 0,
            smcfgr: 0,
            dmaintenr: 0,
            intfr: 0,
            chctlr1: 0,
            chctlr2: 0,
            ccer: 0,
            cnt: 0,
            psc: 0,
            atrlr: u16::MAX,
            rptcr: 0,
            chcv: [0; 4],
            bdtr: 0,
            dmacfgr: 0,
            dmaadr: 0,
            epoch: 0,
        }
    }

    fn interval(&self) -> u64 {
        (u64::from(self.psc) + 1).saturating_mul(u64::from(self.atrlr) + 1)
    }

    fn update(&mut self, now: SimTime) {
        if self.ctlr1 & Self::CEN == 0 {
            return;
        }
        let elapsed = now.ticks().saturating_sub(self.epoch);
        let interval = self.interval();
        if elapsed >= interval {
            let periods = elapsed / interval;
            self.epoch = self.epoch.saturating_add(periods.saturating_mul(interval));
            self.intfr |= Self::UPDATE;
        }
        self.cnt = u16::try_from(
            ((now.ticks().saturating_sub(self.epoch)) / (u64::from(self.psc) + 1))
                .min(u64::from(u16::MAX)),
        )
        .expect("clamped WCH timer counter fits u16");
    }

    fn restart(&mut self, now: SimTime) {
        self.epoch = now.ticks();
        self.cnt = 0;
    }
}

/// Functional WCH TIM2-style general-purpose timer.
///
/// One prescaled counter period advances in deterministic abstract ticks. The
/// model implements the update-event, interrupt-enable, status, prescaler,
/// auto-reload, and counter registers used by the six-chip baseline.
pub struct WchTimer {
    name: String,
    state: Rc<RefCell<WchTimerState>>,
}

impl WchTimer {
    /// Creates a reset timer and its scheduler-facing interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, WchTimerHandle) {
        let state = Rc::new(RefCell::new(WchTimerState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchTimerHandle { state },
        )
    }

    fn require_register_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH TIM requires halfword or word access at a register boundary",
            ));
        }
        Ok(())
    }
}

impl Device for WchTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        Self::require_register_access(offset, width)?;
        let mut state = self.state.borrow_mut();
        state.update(at);
        let value = match offset {
            0x00 => state.ctlr1,
            0x04 => state.ctlr2,
            0x08 => state.smcfgr,
            0x0c => state.dmaintenr,
            0x10 => state.intfr,
            0x14 => 0,
            0x18 => state.chctlr1,
            0x1c => state.chctlr2,
            0x20 => state.ccer,
            0x24 => state.cnt,
            0x28 => state.psc,
            0x2c => state.atrlr,
            0x30 => state.rptcr,
            0x34 | 0x38 | 0x3c | 0x40 => {
                state.chcv[usize::try_from((offset - 0x34) / 4).expect("channel index fits")]
            }
            0x44 => state.bdtr,
            0x48 => state.dmacfgr,
            0x4c => state.dmaadr,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH TIM read at offset {offset:#x}"
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
        Self::require_register_access(offset, width)?;
        let value = u16::try_from(value & u64::from(u16::MAX))
            .expect("masked WCH timer register value fits u16");
        let mut state = self.state.borrow_mut();
        state.update(at);
        match offset {
            0x00 => {
                let was_enabled = state.ctlr1 & WchTimerState::CEN != 0;
                state.ctlr1 = value;
                if !was_enabled && value & WchTimerState::CEN != 0 {
                    state.restart(at);
                }
            }
            0x04 => state.ctlr2 = value,
            0x08 => state.smcfgr = value,
            0x0c => state.dmaintenr = value,
            // WCH's status register clears a flag when software writes zero
            // to that bit; the vendor HAL therefore writes `~flag`.
            0x10 => state.intfr &= value,
            0x14 => {
                if value & WchTimerState::UPDATE != 0 {
                    state.intfr |= WchTimerState::UPDATE;
                    state.restart(at);
                }
            }
            0x18 => state.chctlr1 = value,
            0x1c => state.chctlr2 = value,
            0x20 => state.ccer = value,
            0x24 => {
                state.cnt = value;
                let divider = u64::from(state.psc) + 1;
                state.epoch = at.ticks().saturating_sub(u64::from(value) * divider);
            }
            0x28 => {
                state.psc = value;
                state.restart(at);
            }
            0x2c => {
                state.atrlr = value;
                state.restart(at);
            }
            0x30 => state.rptcr = value,
            0x34 | 0x38 | 0x3c | 0x40 => {
                let channel = usize::try_from((offset - 0x34) / 4).expect("channel index fits");
                state.chcv[channel] = value;
            }
            0x44 => state.bdtr = value,
            0x48 => state.dmacfgr = value,
            0x4c => state.dmaadr = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH TIM write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchTimerState::reset();
    }
}

const WCH_EXTI_MASK: u32 = 0x03ff;

/// Scheduler-facing handle for WCH AFIO-selected external GPIO edges.
#[derive(Clone)]
pub struct WchExtiHandle {
    state: Rc<RefCell<WchExtiState>>,
}

impl WchExtiHandle {
    /// Samples the mapped GPIO ports and returns the masked EXTI request.
    pub fn pending(&self, inputs: [u32; 3]) -> bool {
        let mut state = self.state.borrow_mut();
        for line in 0..8 {
            let port = match (state.exticr >> (line * 2)) & 3 {
                2 => 1,
                3 => 2,
                _ => 0,
            };
            let current = inputs[port] & (1 << line) != 0;
            let previous = state.previous[line];
            if (current && !previous && state.rising & (1 << line) != 0)
                || (!current && previous && state.falling & (1 << line) != 0)
            {
                state.flags |= 1 << line;
            }
            state.previous[line] = current;
        }
        state.flags & state.interrupt_enable & WCH_EXTI_MASK != 0
    }
}

#[derive(Clone)]
struct WchExtiState {
    exticr: u32,
    interrupt_enable: u32,
    event_enable: u32,
    rising: u32,
    falling: u32,
    software: u32,
    flags: u32,
    previous: [bool; 8],
}

impl Default for WchExtiState {
    fn default() -> Self {
        Self {
            exticr: 0,
            interrupt_enable: 0,
            event_enable: 0,
            rising: 0,
            falling: 0,
            software: 0,
            flags: 0,
            previous: [false; 8],
        }
    }
}

/// Functional WCH AFIO remap and external-interrupt routing block.
pub struct WchAfio {
    name: String,
    state: Rc<RefCell<WchExtiState>>,
    pcfr1: u32,
}

impl WchAfio {
    fn new(name: impl Into<String>, state: Rc<RefCell<WchExtiState>>) -> Self {
        Self {
            name: name.into(),
            state,
            pcfr1: 0,
        }
    }
}

impl Device for WchAfio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH AFIO requires aligned word access"));
        }
        let value = match offset {
            0x04 => self.pcfr1,
            0x08 => self.state.borrow().exticr,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH AFIO read at {offset:#x}"
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH AFIO requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("AFIO value fits");
        match offset {
            0x04 => self.pcfr1 = value,
            0x08 => self.state.borrow_mut().exticr = value & 0xffff,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH AFIO write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.pcfr1 = 0;
        self.state.borrow_mut().exticr = 0;
    }
}

/// Functional WCH EXTI edge detector for GPIO lines 0 through 7.
pub struct WchExti {
    name: String,
    state: Rc<RefCell<WchExtiState>>,
}

impl WchExti {
    /// Creates EXTI, its scheduler handle, and the coupled AFIO block.
    pub fn new(
        name: impl Into<String>,
        afio_name: impl Into<String>,
    ) -> (Self, WchExtiHandle, WchAfio) {
        let state = Rc::new(RefCell::new(WchExtiState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchExtiHandle {
                state: state.clone(),
            },
            WchAfio::new(afio_name, state),
        )
    }
}

impl Device for WchExti {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH EXTI requires aligned word access"));
        }
        let state = self.state.borrow();
        let value = match offset {
            0x00 => state.interrupt_enable,
            0x04 => state.event_enable,
            0x08 => state.rising,
            0x0c => state.falling,
            0x10 => state.software,
            0x14 => state.flags,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH EXTI read at {offset:#x}"
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH EXTI requires aligned word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("EXTI value fits") & WCH_EXTI_MASK;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.interrupt_enable = value,
            0x04 => state.event_enable = value,
            0x08 => state.rising = value,
            0x0c => state.falling = value,
            0x10 => {
                state.software = value;
                state.flags |= value;
            }
            0x14 => state.flags &= !value,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH EXTI write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchExtiState::default();
    }
}

/// Shared PFIC state used by the machine scheduler to raise a `QingKe` input.
#[derive(Clone)]
pub struct WchPficHandle {
    state: Rc<RefCell<WchPficState>>,
}

impl WchPficHandle {
    /// Sets one PFIC pending bit from a level-sensitive peripheral source.
    pub fn set_pending(&self, interrupt: u16, pending: bool) {
        if interrupt >= 256 {
            return;
        }
        let mut state = self.state.borrow_mut();
        let word = usize::from(interrupt / 32);
        let bit = 1_u32 << (interrupt % 32);
        if pending {
            state.pending[word] |= bit;
        } else {
            state.pending[word] &= !bit;
        }
    }

    /// Returns the lowest-numbered enabled pending interrupt.
    pub fn next_pending(&self) -> Option<u16> {
        let state = self.state.borrow();
        state
            .enabled
            .iter()
            .zip(state.pending.iter())
            .enumerate()
            .find_map(|(word, (enabled, pending))| {
                let ready = enabled & pending;
                (ready != 0).then(|| {
                    u16::try_from(word).expect("PFIC word index fits u16") * 32
                        + u16::try_from(ready.trailing_zeros()).expect("PFIC bit index fits u16")
                })
            })
    }
}

struct WchPficState {
    enabled: [u32; 8],
    pending: [u32; 8],
    active: [u32; 8],
    threshold: u32,
    config: u32,
    priorities: [u8; 256],
    system_control: u32,
}

impl WchPficState {
    fn reset() -> Self {
        Self {
            enabled: [0; 8],
            pending: [0; 8],
            active: [0; 8],
            threshold: 0,
            config: 0,
            priorities: [0; 256],
            system_control: 0,
        }
    }
}

/// Functional WCH Programmable Fast Interrupt Controller register block.
pub struct WchPfic {
    name: String,
    state: Rc<RefCell<WchPficState>>,
}

impl WchPfic {
    /// Creates a reset PFIC and its scheduler-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, WchPficHandle) {
        let state = Rc::new(RefCell::new(WchPficState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchPficHandle { state },
        )
    }

    fn word_index(offset: u64, base: u64) -> Option<usize> {
        (offset >= base && offset < base + 0x20 && offset % 4 == 0)
            .then(|| usize::try_from((offset - base) / 4).expect("PFIC word index fits"))
    }
}

impl Device for WchPfic {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.borrow();
        if width == AccessWidth::Byte && (0x400..0x500).contains(&offset) {
            let index = usize::try_from(offset - 0x400).expect("PFIC priority index fits");
            return Ok(u64::from(state.priorities[index]));
        }
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH PFIC requires aligned word access"));
        }
        let value = if let Some(index) = Self::word_index(offset, 0x000) {
            state.enabled[index] & state.pending[index]
        } else if let Some(index) = Self::word_index(offset, 0x020) {
            state.pending[index]
        } else if let Some(index) = Self::word_index(offset, 0x100) {
            state.enabled[index]
        } else if let Some(index) = Self::word_index(offset, 0x180) {
            state.enabled[index]
        } else if let Some(index) = Self::word_index(offset, 0x200) {
            state.pending[index]
        } else if let Some(index) = Self::word_index(offset, 0x280) {
            state.pending[index]
        } else if let Some(index) = Self::word_index(offset, 0x300) {
            state.active[index]
        } else {
            match offset {
                0x040 => state.threshold,
                0x048 => state.config,
                0x04c => state
                    .enabled
                    .iter()
                    .zip(state.pending.iter())
                    .enumerate()
                    .find_map(|(word, (enabled, pending))| {
                        let ready = enabled & pending;
                        (ready != 0).then(|| {
                            u32::try_from(word).expect("PFIC word index fits u32") * 32
                                + ready.trailing_zeros()
                        })
                    })
                    .unwrap_or(u32::MAX),
                0xd10 => state.system_control,
                _ if (0x400..0x500).contains(&offset) => {
                    let index = usize::try_from(offset - 0x400).expect("PFIC priority index fits");
                    u32::from_le_bytes([
                        state.priorities[index],
                        state.priorities[index + 1],
                        state.priorities[index + 2],
                        state.priorities[index + 3],
                    ])
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH PFIC read at offset {offset:#x}"
                    )));
                }
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
        let mut state = self.state.borrow_mut();
        if width == AccessWidth::Byte && (0x400..0x500).contains(&offset) {
            let index = usize::try_from(offset - 0x400).expect("PFIC priority index fits");
            state.priorities[index] =
                u8::try_from(value & u64::from(u8::MAX)).expect("masked PFIC priority fits u8");
            return Ok(());
        }
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH PFIC requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked PFIC register value fits u32");
        if let Some(index) = Self::word_index(offset, 0x100) {
            state.enabled[index] |= value;
        } else if let Some(index) = Self::word_index(offset, 0x180) {
            state.enabled[index] &= !value;
        } else if let Some(index) = Self::word_index(offset, 0x200) {
            state.pending[index] |= value;
        } else if let Some(index) = Self::word_index(offset, 0x280) {
            state.pending[index] &= !value;
        } else if let Some(index) = Self::word_index(offset, 0x300) {
            state.active[index] &= !value;
        } else {
            match offset {
                0x040 => state.threshold = value,
                0x048 => state.config = value,
                0xd10 => state.system_control = value,
                _ if (0x400..0x500).contains(&offset) => {
                    let index = usize::try_from(offset - 0x400).expect("PFIC priority index fits");
                    state.priorities[index..index + 4].copy_from_slice(&value.to_le_bytes());
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH PFIC write at offset {offset:#x}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchPficState::reset();
    }
}
