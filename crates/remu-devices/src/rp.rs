use super::*;

/// Functional RP2040 reset controller, including the peripheral atomic-register aliases.
pub struct Rp2040Resets {
    name: String,
    reset: u32,
    watchdog_select: u32,
}

impl Rp2040Resets {
    pub(crate) const VALID_MASK: u32 = 0x01ff_ffff;

    /// Creates a reset controller in the boot-ROM handoff state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reset: 0,
            watchdog_select: 0,
        }
    }

    fn register_value(&self, register: u64) -> Result<u32, DeviceError> {
        match register {
            0x00 => Ok(self.reset),
            0x04 => Ok(self.watchdog_select),
            0x08 => Ok(!self.reset & Self::VALID_MASK),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 RESETS read at offset {register:#x}"
            ))),
        }
    }

    pub(crate) fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Resets {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        Ok(u64::from(self.register_value(offset & 0x0fff)?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let register = offset & 0x0fff;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked RP2040 register value fits");
        match register {
            0x00 => {
                Self::update(&mut self.reset, alias, value)?;
                self.reset &= Self::VALID_MASK;
            }
            0x04 => {
                Self::update(&mut self.watchdog_select, alias, value)?;
                self.watchdog_select &= Self::VALID_MASK;
            }
            0x08 => {
                return Err(DeviceError::new(
                    "RP2040 RESET_DONE is a read-only register",
                ));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 RESETS write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset = 0;
        self.watchdog_select = 0;
    }
}

/// Functional RP2040 clock controller with immediate source selection.
pub struct Rp2040Clocks {
    name: String,
    registers: [u32; 50],
}

impl Rp2040Clocks {
    /// Creates the reset-state clock register bank.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 50];
        for offset in [0x04_usize, 0x10, 0x1c, 0x28, 0x34, 0x40, 0x58, 0x64, 0x70] {
            registers[offset / 4] = 0x100;
        }
        for offset in [
            0x08_usize, 0x14, 0x20, 0x2c, 0x38, 0x44, 0x50, 0x5c, 0x68, 0x74,
        ] {
            registers[offset / 4] = 1;
        }
        Self {
            name: name.into(),
            registers,
        }
    }

    fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 CLOCKS atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Clocks {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        // Source switching is functional and instantaneous. The selected registers remain
        // one-hot because the SDK also uses exact equality while moving through the glitchless
        // reference and system-clock muxes.
        if register_offset == 0x38 {
            return Ok(u64::from(1_u32 << (self.registers[0x30 / 4] & 3)));
        }
        if register_offset == 0x44 {
            return Ok(u64::from(1_u32 << (self.registers[0x3c / 4] & 1)));
        }
        if matches!(
            register_offset,
            0x08 | 0x14 | 0x20 | 0x2c | 0x50 | 0x5c | 0x68 | 0x74
        ) {
            return Ok(1);
        }
        if register_offset == 0xb0 || register_offset == 0xb4 {
            return Ok(u64::from(u32::MAX));
        }
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "unmodeled RP2040 CLOCKS read at offset {register_offset:#x}"
                ))
            })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 CLOCKS write at offset {register_offset:#x}"
            ))
        })?;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked clock register value fits");
        Self::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 crystal oscillator with immediate stabilization.
pub struct Rp2040Xosc {
    name: String,
    control: u32,
    startup: u32,
    count: u32,
}

impl Rp2040Xosc {
    /// Creates a reset-state crystal oscillator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control: 0,
            startup: 0,
            count: 0,
        }
    }
}

impl Device for Rp2040Xosc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control,
            0x04 => 0x8000_1000 | (self.control & 3),
            0x08 => 0,
            0x0c => self.startup,
            0x1c => self.count,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC read at offset {register:#x}"
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked XOSC value fits");
        match offset & 0x0fff {
            0x00 => self.control = value & 0x00ff_ffff,
            0x04 => {}
            0x08 => {}
            0x0c => self.startup = value & 0x0010_3fff,
            0x1c => self.count = value & 0xff,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.control = 0;
        self.startup = 0;
        self.count = 0;
    }
}

/// Functional RP2040 PLL with immediate lock acquisition.
pub struct Rp2040Pll {
    name: String,
    control_status: u32,
    power: u32,
    feedback_divider: u32,
    primitive: u32,
}

impl Rp2040Pll {
    /// Creates a reset-state PLL.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control_status: 1,
            power: 0x2d,
            feedback_divider: 0,
            primitive: 0x0007_7000,
        }
    }
}

impl Device for Rp2040Pll {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control_status | 0x8000_0000,
            0x04 => self.power,
            0x08 => self.feedback_divider,
            0x0c => self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL read at offset {register:#x}"
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked PLL value fits");
        let register = match offset & 0x0fff {
            0x00 => &mut self.control_status,
            0x04 => &mut self.power,
            0x08 => &mut self.feedback_divider,
            0x0c => &mut self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL write at offset {register:#x}"
                )));
            }
        };
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 watchdog and microsecond-tick divider.
pub struct Rp2040Watchdog {
    name: String,
    registers: [u32; 12],
}

impl Rp2040Watchdog {
    /// Creates the watchdog reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 12];
        registers[0] = 0x0700_0000;
        registers[0x2c / 4] = 0x200;
        Self {
            name: name.into(),
            registers,
        }
    }
}

impl Device for Rp2040Watchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small watchdog offset fits");
        let mut value = *self.registers.get(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 WATCHDOG read at offset {register_offset:#x}"
            ))
        })?;
        if register_offset == 0x2c && value & 0x200 != 0 {
            value |= 0x400;
        }
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
            return Err(DeviceError::new(
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        if register_offset == 0x08 {
            return Err(DeviceError::new("RP2040 WATCHDOG REASON is read-only"));
        }
        let index = usize::try_from(register_offset / 4).expect("small watchdog offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 WATCHDOG write at offset {register_offset:#x}"
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked watchdog value fits");
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Shared RP2040 timer interrupt view.
#[derive(Clone)]
pub struct Rp2040TimerHandle {
    state: Rc<RefCell<Rp2040TimerState>>,
}

impl Rp2040TimerHandle {
    /// Returns the four masked alarm interrupt bits at `now`.
    pub fn pending(&self, now: SimTime) -> u8 {
        let mut state = self.state.borrow_mut();
        let previous = state.raw_interrupt;
        state.update(now);
        let pending = (state.raw_interrupt | state.force_interrupt) & state.interrupt_enable;
        if state.raw_interrupt != previous && std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer alarm at={} raw={:#x} enabled={:#x} pending={pending:#x}",
                now.ticks(),
                state.raw_interrupt,
                state.interrupt_enable,
            );
        }
        pending
    }
}

struct Rp2040TimerState {
    alarms: [u32; 4],
    armed: u8,
    raw_interrupt: u8,
    interrupt_enable: u8,
    force_interrupt: u8,
    debug_pause: u32,
    paused: bool,
}

impl Rp2040TimerState {
    fn update(&mut self, now: SimTime) {
        if self.paused {
            return;
        }
        let current = now.ticks() as u32;
        for alarm in 0..4 {
            let mask = 1_u8 << alarm;
            if self.armed & mask != 0 && current.wrapping_sub(self.alarms[alarm]) < 0x8000_0000 {
                self.armed &= !mask;
                self.raw_interrupt |= mask;
            }
        }
    }
}

/// Functional RP2040 64-bit microsecond timer and four alarms.
pub struct Rp2040Timer {
    name: String,
    layout: RpTimerLayout,
    state: Rc<RefCell<Rp2040TimerState>>,
}

/// Register layout implemented by the Raspberry Pi timer block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpTimerLayout {
    /// RP2040 timer layout with interrupt registers beginning at offset `0x34`.
    Rp2040,
    /// RP2350 timer layout with LOCKED/SOURCE and interrupts beginning at `0x3c`.
    Rp2350,
}

impl Rp2040Timer {
    /// Creates the free-running timer and a scheduler-facing handle.
    pub fn new(name: impl Into<String>, layout: RpTimerLayout) -> (Self, Rp2040TimerHandle) {
        let state = Rc::new(RefCell::new(Rp2040TimerState {
            alarms: [0; 4],
            armed: 0,
            raw_interrupt: 0,
            interrupt_enable: 0,
            force_interrupt: 0,
            debug_pause: 7,
            paused: false,
        }));
        let handle = Rp2040TimerHandle {
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                layout,
                state,
            },
            handle,
        )
    }
}

impl Device for Rp2040Timer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let mut state = self.state.borrow_mut();
        state.update(at);
        let ticks = at.ticks();
        let value = match offset & 0x0fff {
            0x00 | 0x08 | 0x24 => (ticks >> 32) as u32,
            0x04 | 0x0c | 0x28 => ticks as u32,
            0x10 | 0x14 | 0x18 | 0x1c => {
                state.alarms
                    [usize::try_from(((offset & 0x0fff) - 0x10) / 4).expect("alarm index fits")]
            }
            0x20 => u32::from(state.armed),
            0x2c => state.debug_pause,
            0x30 => u32::from(state.paused),
            0x34 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.raw_interrupt),
            0x38 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.interrupt_enable),
            0x3c if self.layout == RpTimerLayout::Rp2040 => u32::from(state.force_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2040 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            // RP2350 inserts read-only LOCKED and SOURCE registers before INTR.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => 0,
            0x3c if self.layout == RpTimerLayout::Rp2350 => u32::from(state.raw_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.interrupt_enable),
            0x44 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.force_interrupt),
            0x48 if self.layout == RpTimerLayout::Rp2350 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER read at offset {register:#x}"
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked timer value fits");
        let mut state = self.state.borrow_mut();
        state.update(at);
        if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer write at={} offset={offset:#06x} value={value:#010x}",
                at.ticks()
            );
        }
        let register = offset & 0x0fff;
        let alias = (offset >> 12) & 3;
        let update_alias = |current: u8, value: u8| match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("two-bit alias"),
        };
        match register {
            register @ (0x10 | 0x14 | 0x18 | 0x1c) => {
                let alarm = usize::try_from((register - 0x10) / 4).expect("alarm index fits");
                state.alarms[alarm] = value;
                state.armed |= 1 << alarm;
                state.raw_interrupt &= !(1 << alarm);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer arm alarm={alarm} at={} compare={value:#010x}",
                        at.ticks()
                    );
                }
            }
            0x20 => state.armed &= !(value as u8),
            0x2c => state.debug_pause = value & 6,
            0x30 => state.paused = value & 1 != 0,
            0x34 if self.layout == RpTimerLayout::Rp2040 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x38 if self.layout == RpTimerLayout::Rp2040 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x3c if self.layout == RpTimerLayout::Rp2040 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            // LOCKED is read-only. SOURCE selection is not timing-visible in
            // the functional model because both supported sources advance on
            // the same deterministic simulation timeline.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => {}
            0x3c if self.layout == RpTimerLayout::Rp2350 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x40 if self.layout == RpTimerLayout::Rp2350 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP2350 timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x44 if self.layout == RpTimerLayout::Rp2350 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.alarms = [0; 4];
        state.armed = 0;
        state.raw_interrupt = 0;
        state.interrupt_enable = 0;
        state.force_interrupt = 0;
        state.debug_pause = 7;
        state.paused = false;
    }
}

/// Storage-backed RP2040 APB register slice with atomic XOR, SET, and CLEAR aliases.
///
/// This is used for configuration-only blocks whose values affect observability but do not yet
/// schedule independent events, such as pad and GPIO-function selection registers.
pub struct Rp2040RegisterBank {
    name: String,
    reset_values: Vec<u32>,
    registers: Vec<u32>,
}

impl Rp2040RegisterBank {
    /// Creates a word-addressed register slice initialized from `reset_values`.
    pub fn new(name: impl Into<String>, reset_values: Vec<u32>) -> Self {
        Self {
            name: name.into(),
            registers: reset_values.clone(),
            reset_values,
        }
    }
}

impl Device for Rp2040RegisterBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 register bank requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "{} read outside modeled registers at offset {register_offset:#x}",
                    self.name
                ))
            })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 register bank requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside modeled registers at offset {register_offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked register value fits");
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.clone_from(&self.reset_values);
    }
}

/// Deterministic word-oriented hardware random-number register block.
///
/// Functional MCU models use this at the vendor RNG data-register address.
/// It deliberately provides reproducible pseudo-random words: firmware sees
/// changing entropy-like input while repeat traces remain byte-for-byte
/// stable.
pub struct DeterministicRng {
    name: String,
    data_offset: u64,
    seed: u32,
    state: u32,
}

impl DeterministicRng {
    /// Creates a deterministic RNG block with one readable data register.
    pub fn new(name: impl Into<String>, data_offset: u64, seed: u32) -> Self {
        let seed = if seed == 0 { 0x6d2b_79f5 } else { seed };
        Self {
            name: name.into(),
            data_offset,
            seed,
            state: seed,
        }
    }

    fn next(&mut self) -> u32 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.state = value;
        value
    }
}

impl Device for DeterministicRng {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(if offset == self.data_offset {
            u64::from(self.next())
        } else {
            0
        })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        _value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state = self.seed;
    }
}
