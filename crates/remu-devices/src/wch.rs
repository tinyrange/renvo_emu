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
    const UDIS: u16 = 1 << 1;
    const URS: u16 = 1 << 2;
    const OPM: u16 = 1 << 3;
    const DIR: u16 = 1 << 4;
    const CMS_MASK: u16 = 0x60;
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
            if self.ctlr1 & Self::UDIS == 0 {
                self.intfr |= Self::UPDATE;
                if self.ctlr1 & Self::OPM != 0 {
                    self.ctlr1 &= !Self::CEN;
                }
            }
        }
        let phase = u16::try_from(
            ((now.ticks().saturating_sub(self.epoch)) / (u64::from(self.psc) + 1))
                .min(u64::from(u16::MAX)),
        )
        .expect("clamped WCH timer counter fits u16");
        self.cnt = if self.ctlr1 & Self::CMS_MASK == 0 && self.ctlr1 & Self::DIR != 0 {
            self.atrlr.saturating_sub(phase)
        } else {
            phase
        };
    }

    fn restart(&mut self, now: SimTime) {
        self.epoch = now.ticks();
        self.cnt = 0;
    }
}

/// Functional WCH general-purpose/advanced timer register subset.
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
                    state.restart(at);
                    if state.ctlr1 & WchTimerState::UDIS == 0
                        && state.ctlr1 & WchTimerState::URS == 0
                    {
                        state.intfr |= WchTimerState::UPDATE;
                    }
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
                // CHxCVR is a 32-bit register, but only the low 16-bit
                // compare/capture value is writable. Bit 16 is a
                // read-only capture-level indication and the upper bits are
                // reserved on both CH32V003 and CH32V006.
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
pub(crate) const WCH_AFIO_PCFR1_MASK: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (0b11 << 6)
    | (0b11 << 8)
    | (1 << 15)
    | (1 << 17)
    | (1 << 18)
    | (1 << 21)
    | (1 << 22)
    | (1 << 23)
    | (0b111 << 24);

/// Native AFIO register identifiers for the CH32V00x window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WchAfioRegister {
    /// Remap and alternate-function selection.
    Pcfr1,
    /// EXTI0-7 port selection.
    Exticr,
}

impl WchAfioRegister {
    /// Returns the register's offset within the AFIO window.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Pcfr1 => 0x04,
            Self::Exticr => 0x08,
        }
    }
}

impl TryFrom<u64> for WchAfioRegister {
    type Error = ();

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x04 => Ok(Self::Pcfr1),
            0x08 => Ok(Self::Exticr),
            _ => Err(()),
        }
    }
}

/// Native EXTI register identifiers for the CH32V00x window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WchExtiRegister {
    /// Interrupt enable register.
    InterruptEnable,
    /// Event enable register.
    EventEnable,
    /// Rising-edge trigger enable register.
    RisingTrigger,
    /// Falling-edge trigger enable register.
    FallingTrigger,
    /// Software interrupt/event register.
    SoftwareTrigger,
    /// Interrupt flag register.
    InterruptFlag,
}

impl WchExtiRegister {
    /// Returns the register's offset within the EXTI window.
    pub const fn offset(self) -> u64 {
        match self {
            Self::InterruptEnable => 0x00,
            Self::EventEnable => 0x04,
            Self::RisingTrigger => 0x08,
            Self::FallingTrigger => 0x0c,
            Self::SoftwareTrigger => 0x10,
            Self::InterruptFlag => 0x14,
        }
    }
}

impl TryFrom<u64> for WchExtiRegister {
    type Error = ();

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::InterruptEnable),
            0x04 => Ok(Self::EventEnable),
            0x08 => Ok(Self::RisingTrigger),
            0x0c => Ok(Self::FallingTrigger),
            0x10 => Ok(Self::SoftwareTrigger),
            0x14 => Ok(Self::InterruptFlag),
            _ => Err(()),
        }
    }
}

/// Scheduler-facing state for the WCH AFIO-selected external interrupt lines.
#[derive(Clone)]
pub struct WchExtiHandle {
    state: Rc<RefCell<WchExtiState>>,
}

impl WchExtiHandle {
    /// Samples GPIO ports and latches enabled rising/falling edges on EXTI0-7.
    ///
    /// The array is ordered as PA, PC, and PD, matching the GPIO ports exposed
    /// by the CH32V003/CH32V006 machine model. AFIO EXTICR selects which port
    /// supplies each EXTI line.
    pub fn pending(&self, inputs: [u32; 3]) -> bool {
        let mut state = self.state.borrow_mut();
        for line in 0..8 {
            let port = match (state.exticr >> (line * 2)) & 3 {
                0 => Some(0), // PA
                2 => Some(1), // PC
                3 => Some(2), // PD
                // The CH32V00x AFIO encoding 01 is reserved. It must not
                // silently alias an unmodelled PB port to PA.
                _ => None,
            };
            let current = port.is_some_and(|port| inputs[port] & (1 << line) != 0);
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

/// Functional WCH AFIO remap and EXTI edge-routing register blocks.
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
        let register = WchAfioRegister::try_from(offset).map_err(|()| {
            DeviceError::new(format!("unmodeled WCH AFIO read at offset {offset:#x}"))
        })?;
        let value = match register {
            WchAfioRegister::Pcfr1 => self.pcfr1,
            WchAfioRegister::Exticr => self.state.borrow().exticr,
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
        let register = WchAfioRegister::try_from(offset).map_err(|()| {
            DeviceError::new(format!("unmodeled WCH AFIO write at offset {offset:#x}"))
        })?;
        match register {
            WchAfioRegister::Pcfr1 => self.pcfr1 = value & WCH_AFIO_PCFR1_MASK,
            // The V00x devices expose EXTI0-7 selection in the low 16 bits.
            WchAfioRegister::Exticr => self.state.borrow_mut().exticr = value & 0xffff,
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
        let register = WchExtiRegister::try_from(offset).map_err(|()| {
            DeviceError::new(format!("unmodeled WCH EXTI read at offset {offset:#x}"))
        })?;
        let state = self.state.borrow();
        let value = match register {
            WchExtiRegister::InterruptEnable => state.interrupt_enable,
            WchExtiRegister::EventEnable => state.event_enable,
            WchExtiRegister::RisingTrigger => state.rising,
            WchExtiRegister::FallingTrigger => state.falling,
            WchExtiRegister::SoftwareTrigger => state.software,
            WchExtiRegister::InterruptFlag => state.flags,
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
        let register = WchExtiRegister::try_from(offset).map_err(|()| {
            DeviceError::new(format!("unmodeled WCH EXTI write at offset {offset:#x}"))
        })?;
        match register {
            WchExtiRegister::InterruptEnable => state.interrupt_enable = value,
            WchExtiRegister::EventEnable => state.event_enable = value,
            WchExtiRegister::RisingTrigger => state.rising = value,
            WchExtiRegister::FallingTrigger => state.falling = value,
            WchExtiRegister::SoftwareTrigger => {
                state.software = value;
                state.flags |= value;
            }
            // CH32's INTFR is write-one-to-clear.
            WchExtiRegister::InterruptFlag => state.flags &= !value,
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

    /// Advances the QingKe system-count block and reports its IRQ source.
    pub fn take_systick_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        WchPfic::advance_stk(&mut state, now);
        (state.stk_control & (1 << 31) != 0)
            || (state.stk_control & (1 << 1) != 0 && state.stk_countflag)
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
    stk_control: u32,
    stk_countflag: bool,
    stk_counter: u32,
    stk_compare: u32,
    stk_last_tick: u64,
    stk_clock_remainder: u64,
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
            stk_control: 0,
            stk_countflag: false,
            stk_counter: 0,
            stk_compare: 0,
            stk_last_tick: 0,
            stk_clock_remainder: 0,
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

    fn advance_stk(state: &mut WchPficState, now: SimTime) {
        let elapsed = now.ticks().saturating_sub(state.stk_last_tick);
        state.stk_last_tick = now.ticks();
        if state.stk_control & 1 == 0 || elapsed == 0 {
            return;
        }
        let divisor = if state.stk_control & (1 << 2) != 0 {
            1
        } else {
            8
        };
        let clocks = state.stk_clock_remainder.saturating_add(elapsed);
        let ticks = clocks / divisor;
        state.stk_clock_remainder = clocks % divisor;
        if ticks == 0 {
            return;
        }
        let period = 1_u64 << 32;
        let compare = u64::from(state.stk_compare);
        if state.stk_control & (1 << 3) != 0 {
            let period = compare + 1;
            let counter = u64::from(state.stk_counter);
            let first = if counter < compare {
                compare - counter
            } else {
                period + compare - counter
            };
            if ticks >= first {
                state.stk_countflag = true;
            }
            state.stk_counter = u32::try_from(((counter % period) + (ticks % period)) % period)
                .expect("STK counter fits u32");
        } else {
            // The QingKe V2 manual describes a vendor-specific up/down path
            // when STRE is clear. Keep the baseline deterministic and bounded
            // with its 32-bit free-running counter semantics.
            let increment = u32::try_from(ticks % period).expect("STK tick count fits u32");
            let counter = u64::from(state.stk_counter);
            let first = if counter < compare {
                compare - counter
            } else {
                period + compare - counter
            };
            if ticks >= first {
                state.stk_countflag = true;
            }
            state.stk_counter = state.stk_counter.wrapping_add(increment);
        }
    }
}

impl Device for WchPfic {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.borrow_mut();
        Self::advance_stk(&mut state, at);
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
                0x1000 => state.stk_control,
                0x1004 => u32::from(state.stk_countflag),
                0x1008 => state.stk_counter,
                0x1010 => state.stk_compare,
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
        at: SimTime,
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
        Self::advance_stk(&mut state, at);
        if offset == 0x1000 {
            state.stk_control = value & ((1 << 31) | 0x0f);
        } else if offset == 0x1004 {
            if value & 1 == 0 {
                state.stk_countflag = false;
            }
        } else if offset == 0x1008 {
            state.stk_counter = value;
        } else if offset == 0x1010 {
            state.stk_compare = value;
        } else if let Some(index) = Self::word_index(offset, 0x100) {
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

const SPI_CR1_MSTR: u32 = 1 << 2;
const SPI_CR1_SPE: u32 = 1 << 6;
const SPI_CR2_RXNEIE: u32 = 1 << 6;
const SPI_CR2_TXEIE: u32 = 1 << 7;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;

#[derive(Clone)]
/// Host-facing state for a CH32V00x SPI1 instance.
pub struct WchSpiHandle {
    state: Rc<RefCell<WchSpiState>>,
}

impl WchSpiHandle {
    /// Captured MOSI bytes from completed SPI1 transfers.
    pub fn tx_bytes(&self) -> Vec<u8> {
        self.state.borrow().tx.clone()
    }

    /// Supplies the MISO byte returned by the next SPI1 transfer.
    pub fn inject_rx(&self, value: u8) {
        self.state.borrow_mut().incoming.push(value);
    }

    /// Reports whether an enabled SPI status flag requests the PFIC line.
    pub fn pending(&self) -> bool {
        let state = self.state.borrow();
        state.cr1 & SPI_CR1_SPE != 0
            && ((state.sr & SPI_SR_RXNE != 0 && state.cr2 & SPI_CR2_RXNEIE != 0)
                || (state.sr & SPI_SR_TXE != 0 && state.cr2 & SPI_CR2_TXEIE != 0))
    }
}

struct WchSpiState {
    cr1: u32,
    cr2: u32,
    sr: u32,
    dr: u32,
    tx: Vec<u8>,
    incoming: Vec<u8>,
    hub: SignalHub,
    tx_signal: SignalId,
    rx_signal: SignalId,
    strobe_signal: SignalId,
    strobe: bool,
}

impl WchSpiState {
    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed WCH SPI signal width is valid"),
                at,
            )
            .expect("WCH SPI signal identity is fixed at construction");
    }

    fn reset(&mut self, at: SimTime) {
        self.cr1 = 0;
        self.cr2 = 0;
        self.sr = SPI_SR_TXE;
        self.dr = 0;
        self.tx.clear();
        self.incoming.clear();
        self.strobe = false;
        self.set_signal(self.tx_signal, 0, 8, at);
        self.set_signal(self.rx_signal, 0, 8, at);
        self.set_signal(self.strobe_signal, 0, 1, at);
    }
}

/// Functional CH32V00x SPI1 register slice.
pub struct WchSpi {
    name: String,
    state: Rc<RefCell<WchSpiState>>,
}

impl WchSpi {
    /// Creates SPI1 with native signals rooted at `board.<name>`.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, WchSpiHandle), SignalError> {
        let name = name.into();
        let tx_signal = hub.declare(
            format!("board.{name}.tx_byte"),
            SignalValue::from_u64(0, 8)?,
            Some("CH32 SPI1 MOSI byte".to_owned()),
        )?;
        let rx_signal = hub.declare(
            format!("board.{name}.rx_byte"),
            SignalValue::from_u64(0, 8)?,
            Some("CH32 SPI1 MISO byte".to_owned()),
        )?;
        let strobe_signal = hub.declare(
            format!("board.{name}.tx_strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("CH32 SPI1 transfer event".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(WchSpiState {
            cr1: 0,
            cr2: 0,
            sr: SPI_SR_TXE,
            dr: 0,
            tx: Vec::new(),
            incoming: Vec::new(),
            hub,
            tx_signal,
            rx_signal,
            strobe_signal,
            strobe: false,
        }));
        state.borrow_mut().reset(SimTime::ZERO);
        Ok((
            Self {
                name,
                state: state.clone(),
            },
            WchSpiHandle { state },
        ))
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH SPI requires aligned halfword or word access",
            ));
        }
        Ok(())
    }
}

impl Device for WchSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let mut state = self.state.borrow_mut();
        let value = match offset {
            0x00 => state.cr1,
            0x04 => state.cr2,
            0x08 => state.sr,
            0x0c => {
                let value = state.dr;
                state.sr &= !SPI_SR_RXNE;
                value
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH SPI read at offset {offset:#x}"
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
        Self::require_access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked WCH SPI register value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.cr1 = value,
            0x04 => state.cr2 = value,
            0x08 => state.sr &= value | SPI_SR_TXE,
            0x0c => {
                if state.cr1 & (SPI_CR1_SPE | SPI_CR1_MSTR) == (SPI_CR1_SPE | SPI_CR1_MSTR) {
                    let tx = value as u8;
                    let rx = state.incoming.first().copied().unwrap_or(tx);
                    if !state.incoming.is_empty() {
                        state.incoming.remove(0);
                    }
                    state.tx.push(tx);
                    state.dr = u32::from(rx);
                    state.sr |= SPI_SR_RXNE | SPI_SR_TXE;
                    state.set_signal(state.tx_signal, u64::from(tx), 8, at);
                    state.set_signal(state.rx_signal, u64::from(rx), 8, at);
                    state.strobe = !state.strobe;
                    state.set_signal(state.strobe_signal, u64::from(state.strobe), 1, at);
                }
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH SPI write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset(SimTime::ZERO);
    }
}

#[cfg(test)]
mod spi_tests {
    use super::{SPI_CR1_MSTR, SPI_CR1_SPE, SPI_CR2_RXNEIE, SPI_CR2_TXEIE, WchSpi};
    use crate::SignalHub;
    use remu_bus::Device;
    use remu_core::{AccessWidth, SimTime};

    #[test]
    fn spi1_master_transfer_returns_injected_miso_and_raises_pending() {
        let (mut spi, handle) = WchSpi::new("ch32v003.spi1", SignalHub::new()).unwrap();
        spi.write(
            0x00,
            AccessWidth::Word,
            u64::from(SPI_CR1_MSTR | SPI_CR1_SPE),
            SimTime::ZERO,
        )
        .unwrap();
        spi.write(
            0x04,
            AccessWidth::Word,
            u64::from(SPI_CR2_RXNEIE | SPI_CR2_TXEIE),
            SimTime::ZERO,
        )
        .unwrap();
        handle.inject_rx(0xa5);
        spi.write(0x0c, AccessWidth::Word, 0x3c, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.tx_bytes(), [0x3c]);
        assert!(handle.pending());
        assert_eq!(spi.read(0x0c, AccessWidth::Word, SimTime::ZERO), Ok(0xa5));
    }
}
