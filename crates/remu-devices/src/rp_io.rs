use super::*;

const GPIO_COUNT: usize = 48;
const EVENT_GROUP_COUNT: usize = 6;
const EDGE_MASK: u32 = 0xcccc_cccc;

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
        for pin in 0..usize::from(pins).min(GPIO_COUNT).min(gpio.pin_count()) {
            previous_input[pin] = gpio
                .resolved(u8::try_from(pin).expect("GPIO index fits u8"))
                .is_ok_and(|value| value == Logic::One);
        }
        let state = Rc::new(RefCell::new(RpIoBankState {
            gpio,
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
        let pins = state.gpio.pin_count().min(GPIO_COUNT);
        for pin in 0..pins {
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
            let high = pin < self.gpio.pin_count() && self.input_level(pin);
            events &= !(0x3_u32 << shift);
            events |= 1_u32 << (shift + usize::from(!high));
        }
        events
    }

    fn status(&self, pin: usize) -> u32 {
        if pin >= self.gpio.pin_count() {
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

    fn register_group(register: u64, first: u64) -> Option<usize> {
        (register >= first && register < first + 4 * EVENT_GROUP_COUNT as u64)
            .then(|| usize::try_from((register - first) / 4).expect("IO_BANK0 group fits"))
    }

    fn reset(&mut self) {
        self.controls = [0x1f; GPIO_COUNT];
        self.edge_latches = [0; EVENT_GROUP_COUNT];
        self.proc0_enable = [0; EVENT_GROUP_COUNT];
        self.proc0_force = [0; EVENT_GROUP_COUNT];
        self.proc1_enable = [0; EVENT_GROUP_COUNT];
        self.proc1_force = [0; EVENT_GROUP_COUNT];
        for pin in 0..GPIO_COUNT.min(self.gpio.pin_count()) {
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
        let register = offset & 0x0fff;
        let state = self.state.borrow();
        let value = if register < 0x180 {
            let pin = usize::try_from(register / 8).expect("GPIO index fits");
            match register & 7 {
                0 => state.status(pin),
                4 => state.controls.get(pin).copied().unwrap_or(0x1f),
                _ => unreachable!(),
            }
        } else if (0x200..0x230).contains(&register) {
            let pin = usize::try_from((register - 0x200) / 4).expect("GPIO index fits");
            u32::from(state.status(pin) & (1 << 26) != 0)
        } else if let Some(group) = RpIoBankState::register_group(register, 0x230) {
            state.raw_events(group)
        } else if let Some(group) = RpIoBankState::register_group(register, 0x248) {
            state.proc0_enable[group]
        } else if let Some(group) = RpIoBankState::register_group(register, 0x260) {
            state.proc0_force[group]
        } else if let Some(group) = RpIoBankState::register_group(register, 0x278) {
            state.group_value(group, true, false)
        } else if let Some(group) = RpIoBankState::register_group(register, 0x290) {
            state.proc1_enable[group]
        } else if let Some(group) = RpIoBankState::register_group(register, 0x2a8) {
            state.proc1_force[group]
        } else if let Some(group) = RpIoBankState::register_group(register, 0x2c0) {
            state.group_value(group, false, false)
        } else {
            0
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
        let register = offset & 0x0fff;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("IO_BANK0 value fits");
        let mut state = self.state.borrow_mut();
        if register < 0x180 && register & 7 == 4 {
            let pin = usize::try_from(register / 8).expect("GPIO index fits");
            if let Some(control) = state.controls.get_mut(pin) {
                RpIoBankState::atomic_update(control, alias, value & 0x3003_f01f)?;
            }
        } else if let Some(group) = RpIoBankState::register_group(register, 0x230) {
            state.edge_latches[group] &= !(value & EDGE_MASK);
        } else if let Some(group) = RpIoBankState::register_group(register, 0x248) {
            RpIoBankState::atomic_update(&mut state.proc0_enable[group], alias, value)?;
        } else if let Some(group) = RpIoBankState::register_group(register, 0x260) {
            RpIoBankState::atomic_update(&mut state.proc0_force[group], alias, value)?;
        } else if let Some(group) = RpIoBankState::register_group(register, 0x290) {
            RpIoBankState::atomic_update(&mut state.proc1_enable[group], alias, value)?;
        } else if let Some(group) = RpIoBankState::register_group(register, 0x2a8) {
            RpIoBankState::atomic_update(&mut state.proc1_force[group], alias, value)?;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}
