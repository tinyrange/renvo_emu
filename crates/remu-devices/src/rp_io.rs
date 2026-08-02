use super::*;

const GPIO_COUNT: usize = 48;
const EVENT_GROUP_COUNT: usize = 6;
const EDGE_MASK: u32 = 0xcccc_cccc;

/// Security/processor bank represented by an RP2350 IO_BANK0 IRQ summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpIoBankSummary {
    /// PROC0 secure summary.
    Proc0Secure,
    /// PROC0 non-secure summary.
    Proc0NonSecure,
    /// PROC1 secure summary.
    Proc1Secure,
    /// PROC1 non-secure summary.
    Proc1NonSecure,
    /// Dormant-wake secure summary (not functionally generated yet).
    ComaWakeSecure,
    /// Dormant-wake non-secure summary (not functionally generated yet).
    ComaWakeNonSecure,
}

impl RpIoBankSummary {
    const fn base(self) -> u64 {
        match self {
            Self::Proc0Secure => 0x200,
            Self::Proc0NonSecure => 0x208,
            Self::Proc1Secure => 0x210,
            Self::Proc1NonSecure => 0x218,
            Self::ComaWakeSecure => 0x220,
            Self::ComaWakeNonSecure => 0x228,
        }
    }

    const fn processor(self) -> Option<bool> {
        match self {
            Self::Proc0Secure | Self::Proc0NonSecure => Some(true),
            Self::Proc1Secure | Self::Proc1NonSecure => Some(false),
            Self::ComaWakeSecure | Self::ComaWakeNonSecure => None,
        }
    }
}

/// RP2350 IO_BANK0 register identifiers.
///
/// Indexed variants correspond to the six packed interrupt groups or the
/// per-pin STATUS/CTRL pairs in the native register map.  Keeping offsets in
/// this enum prevents callers from having to duplicate undocumented numbers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpIoBankRegister {
    /// Per-pin STATUS register.
    GpioStatus(usize),
    /// Per-pin CTRL register.
    GpioControl(usize),
    /// Per-processor/security IRQ summary register.
    IrqSummary {
        /// Summary bank kind.
        kind: RpIoBankSummary,
        /// GPIO bank, zero for GPIO0..31 or one for GPIO32..47.
        bank: usize,
    },
    /// Packed raw interrupt register.
    RawInterrupt(usize),
    /// PROC0 interrupt enable register.
    Proc0Enable(usize),
    /// PROC0 interrupt force register.
    Proc0Force(usize),
    /// PROC0 masked/forced status register.
    Proc0Status(usize),
    /// PROC1 interrupt enable register.
    Proc1Enable(usize),
    /// PROC1 interrupt force register.
    Proc1Force(usize),
    /// PROC1 masked/forced status register.
    Proc1Status(usize),
}

impl RpIoBankRegister {
    const fn group_at(offset: u64, base: u64) -> Option<usize> {
        if offset >= base && offset < base + (EVENT_GROUP_COUNT as u64 * 4) {
            Some(((offset - base) / 4) as usize)
        } else {
            None
        }
    }

    /// Converts a native IO_BANK0 byte offset to a typed register ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset < 0x180 {
            let pin = (offset / 8) as usize;
            return match offset & 7 {
                0 => Some(Self::GpioStatus(pin)),
                4 => Some(Self::GpioControl(pin)),
                _ => None,
            };
        }
        if offset >= 0x200 && offset < 0x230 {
            let kind = match offset & !7 {
                0x200 => RpIoBankSummary::Proc0Secure,
                0x208 => RpIoBankSummary::Proc0NonSecure,
                0x210 => RpIoBankSummary::Proc1Secure,
                0x218 => RpIoBankSummary::Proc1NonSecure,
                0x220 => RpIoBankSummary::ComaWakeSecure,
                0x228 => RpIoBankSummary::ComaWakeNonSecure,
                _ => return None,
            };
            let bank = ((offset - kind.base()) / 4) as usize;
            return Some(Self::IrqSummary { kind, bank });
        }
        if let Some(group) = Self::group_at(offset, 0x230) {
            return Some(Self::RawInterrupt(group));
        }
        if let Some(group) = Self::group_at(offset, 0x248) {
            return Some(Self::Proc0Enable(group));
        }
        if let Some(group) = Self::group_at(offset, 0x260) {
            return Some(Self::Proc0Force(group));
        }
        if let Some(group) = Self::group_at(offset, 0x278) {
            return Some(Self::Proc0Status(group));
        }
        if let Some(group) = Self::group_at(offset, 0x290) {
            return Some(Self::Proc1Enable(group));
        }
        if let Some(group) = Self::group_at(offset, 0x2a8) {
            return Some(Self::Proc1Force(group));
        }
        if let Some(group) = Self::group_at(offset, 0x2c0) {
            return Some(Self::Proc1Status(group));
        }
        None
    }

    /// Returns the native byte offset represented by this register ID.
    pub const fn offset(self) -> u64 {
        match self {
            Self::GpioStatus(pin) => (pin as u64) * 8,
            Self::GpioControl(pin) => (pin as u64) * 8 + 4,
            Self::IrqSummary { kind, bank } => kind.base() + (bank as u64) * 4,
            Self::RawInterrupt(group) => 0x230 + (group as u64) * 4,
            Self::Proc0Enable(group) => 0x248 + (group as u64) * 4,
            Self::Proc0Force(group) => 0x260 + (group as u64) * 4,
            Self::Proc0Status(group) => 0x278 + (group as u64) * 4,
            Self::Proc1Enable(group) => 0x290 + (group as u64) * 4,
            Self::Proc1Force(group) => 0x2a8 + (group as u64) * 4,
            Self::Proc1Status(group) => 0x2c0 + (group as u64) * 4,
        }
    }
}

/// RP2350 IO_BANK0 GPIO status, override, and interrupt state.
///
/// The model intentionally concentrates on the register surface used by the
/// SDK: per-pin STATUS/CTRL, raw INTR, and PROC0/PROC1 enable/force/status
/// registers. Pad electrical muxing and the secure/non-secure interrupt bank
/// remain outside this functional slice.
pub struct RpIoBank {
    name: String,
    state: Rc<RefCell<RpIoBankState>>,
}

/// Scheduler-facing view of RP2350 IO_BANK0 interrupt state.
#[derive(Clone)]
pub struct RpIoBankHandle {
    state: Rc<RefCell<RpIoBankState>>,
}

struct RpIoBankState {
    gpio: GpioHandle,
    pins: usize,
    controls: [u32; GPIO_COUNT],
    edge_latches: [u32; EVENT_GROUP_COUNT],
    proc0_enable: [u32; EVENT_GROUP_COUNT],
    proc0_force: [u32; EVENT_GROUP_COUNT],
    proc1_enable: [u32; EVENT_GROUP_COUNT],
    proc1_force: [u32; EVENT_GROUP_COUNT],
    previous_input: [bool; GPIO_COUNT],
}

impl RpIoBank {
    /// Creates the RP2350 IO_BANK0 slice and a scheduler-facing handle.
    pub fn new(name: impl Into<String>, gpio: GpioHandle, pins: u8) -> (Self, RpIoBankHandle) {
        let mut previous_input = [false; GPIO_COUNT];
        let pins = usize::from(pins).min(GPIO_COUNT).min(gpio.pin_count());
        for pin in 0..pins {
            previous_input[pin] = gpio
                .resolved(u8::try_from(pin).expect("GPIO index fits u8"))
                .is_ok_and(|value| value == Logic::One);
        }
        let state = Rc::new(RefCell::new(RpIoBankState {
            gpio,
            pins,
            controls: [0x1f; GPIO_COUNT],
            edge_latches: [0; EVENT_GROUP_COUNT],
            proc0_enable: [0; EVENT_GROUP_COUNT],
            proc0_force: [0; EVENT_GROUP_COUNT],
            proc1_enable: [0; EVENT_GROUP_COUNT],
            proc1_force: [0; EVENT_GROUP_COUNT],
            previous_input,
        }));
        let handle = RpIoBankHandle {
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                state,
            },
            handle,
        )
    }
}

impl RpIoBankHandle {
    /// Samples GPIO inputs, latches edge events, and returns PROC0 pending.
    pub fn poll(&self, _at: SimTime) -> Result<bool, DeviceError> {
        let mut state = self.state.borrow_mut();
        for pin in 0..state.pins {
            let pin_u8 = u8::try_from(pin).expect("GPIO index fits u8");
            let high = state.gpio.resolved(pin_u8)? == Logic::One;
            let previous = state.previous_input[pin];
            if high != previous {
                let group = pin / 8;
                let shift = (pin % 8) * 4;
                state.edge_latches[group] |= 1_u32 << (shift + if high { 3 } else { 2 });
                state.previous_input[pin] = high;
            }
        }
        Ok(state.proc0_pending())
    }

    /// Returns whether a PROC0 interrupt is currently asserted.
    pub fn pending(&self) -> bool {
        self.state.borrow().proc0_pending()
    }
}

impl RpIoBankState {
    fn group_value(&self, group: usize, proc0: bool, force: bool) -> u32 {
        let raw = self.raw_events(group);
        let enabled = if proc0 {
            self.proc0_enable[group]
        } else {
            self.proc1_enable[group]
        };
        let forced = if proc0 {
            self.proc0_force[group]
        } else {
            self.proc1_force[group]
        };
        if force {
            forced
        } else {
            (raw & enabled) | forced
        }
    }

    fn proc0_pending(&self) -> bool {
        (0..EVENT_GROUP_COUNT).any(|group| self.group_value(group, true, false) != 0)
    }

    fn irq_summary(&self, kind: RpIoBankSummary, bank: usize) -> u32 {
        let Some(proc0) = kind.processor() else {
            // Dormant-wake routing is intentionally outside this functional
            // slice; its summary registers remain clear.
            return 0;
        };
        let first_pin = bank * 32;
        let last_pin = (first_pin + 32).min(GPIO_COUNT);
        let mut summary = 0_u32;
        for pin in first_pin..last_pin {
            let group = pin / 8;
            let shift = (pin % 8) * 4;
            if self.group_value(group, proc0, false) & (0xf << shift) != 0 {
                summary |= 1_u32 << (pin - first_pin);
            }
        }
        summary
    }

    fn input_level(&self, pin: usize) -> bool {
        self.gpio
            .resolved(u8::try_from(pin).expect("GPIO index fits u8"))
            .is_ok_and(|value| value == Logic::One)
    }

    fn output_level(&self, pin: usize) -> bool {
        self.gpio.output() & (1_u32 << pin) != 0
    }

    fn output_enable(&self, pin: usize) -> bool {
        self.gpio.direction() & (1_u32 << pin) != 0
    }

    fn override_value(value: bool, mode: u32) -> bool {
        match mode & 3 {
            0 => value,
            1 => !value,
            2 => false,
            3 => true,
            _ => unreachable!(),
        }
    }

    fn raw_events(&self, group: usize) -> u32 {
        let mut events = self.edge_latches[group];
        for pin_in_group in 0..8 {
            let pin = group * 8 + pin_in_group;
            if pin >= GPIO_COUNT {
                break;
            }
            let shift = pin_in_group * 4;
            let high = pin < self.pins && self.input_level(pin);
            events &= !(0x3_u32 << shift);
            events |= 1_u32 << (shift + if high { 1 } else { 0 });
        }
        events
    }

    fn status(&self, pin: usize) -> u32 {
        if pin >= self.pins {
            return 0;
        }
        let control = self.controls[pin];
        let input = self.input_level(pin);
        let output = Self::override_value(self.output_level(pin), control >> 12);
        let output_enable = Self::override_value(self.output_enable(pin), control >> 14);
        let irq = Self::override_value(
            self.raw_events(pin / 8) & (0xf << ((pin % 8) * 4)) != 0,
            control >> 28,
        );
        u32::from(input) << 17
            | u32::from(output_enable) << 13
            | u32::from(output) << 9
            | u32::from(irq) << 26
    }

    fn atomic_update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2350 IO_BANK0 atomic alias")),
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.controls = [0x1f; GPIO_COUNT];
        self.edge_latches = [0; EVENT_GROUP_COUNT];
        self.proc0_enable = [0; EVENT_GROUP_COUNT];
        self.proc0_force = [0; EVENT_GROUP_COUNT];
        self.proc1_enable = [0; EVENT_GROUP_COUNT];
        self.proc1_force = [0; EVENT_GROUP_COUNT];
        for pin in 0..self.pins {
            self.previous_input[pin] = self.input_level(pin);
        }
    }
}

impl Device for RpIoBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2350 IO_BANK0 requires aligned word access",
            ));
        }
        let register = RpIoBankRegister::from_offset(offset & 0x0fff);
        let state = self.state.borrow();
        let value = match register {
            Some(RpIoBankRegister::GpioStatus(pin)) => state.status(pin),
            Some(RpIoBankRegister::GpioControl(pin)) => {
                state.controls.get(pin).copied().unwrap_or(0x1f)
            }
            Some(RpIoBankRegister::IrqSummary { kind, bank }) => state.irq_summary(kind, bank),
            Some(RpIoBankRegister::RawInterrupt(group)) => state.raw_events(group),
            Some(RpIoBankRegister::Proc0Enable(group)) => state.proc0_enable[group],
            Some(RpIoBankRegister::Proc0Force(group)) => state.proc0_force[group],
            Some(RpIoBankRegister::Proc0Status(group)) => state.group_value(group, true, false),
            Some(RpIoBankRegister::Proc1Enable(group)) => state.proc1_enable[group],
            Some(RpIoBankRegister::Proc1Force(group)) => state.proc1_force[group],
            Some(RpIoBankRegister::Proc1Status(group)) => state.group_value(group, false, false),
            None => 0,
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
            return Err(DeviceError::new(
                "RP2350 IO_BANK0 requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register = RpIoBankRegister::from_offset(offset & 0x0fff);
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("IO_BANK0 value fits");
        let mut state = self.state.borrow_mut();
        match register {
            Some(RpIoBankRegister::GpioControl(pin)) => {
                if let Some(control) = state.controls.get_mut(pin) {
                    RpIoBankState::atomic_update(control, alias, value & 0x3003_f01f)?;
                }
            }
            Some(RpIoBankRegister::RawInterrupt(group)) => {
                state.edge_latches[group] &= !(value & EDGE_MASK);
            }
            Some(RpIoBankRegister::Proc0Enable(group)) => {
                RpIoBankState::atomic_update(&mut state.proc0_enable[group], alias, value)?;
            }
            Some(RpIoBankRegister::Proc0Force(group)) => {
                RpIoBankState::atomic_update(&mut state.proc0_force[group], alias, value)?;
            }
            Some(RpIoBankRegister::Proc1Enable(group)) => {
                RpIoBankState::atomic_update(&mut state.proc1_enable[group], alias, value)?;
            }
            Some(RpIoBankRegister::Proc1Force(group)) => {
                RpIoBankState::atomic_update(&mut state.proc1_force[group], alias, value)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}
