use super::*;

/// RP pad-bank register generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpPadsVariant {
    /// RP2040 eight-bit GPIO pad controls.
    Rp2040,
    /// RP2350 nine-bit GPIO pad controls.
    Rp2350,
}

struct RpPadsState {
    variant: RpPadsVariant,
    voltage_select: u32,
    controls: Vec<u32>,
}

/// Functional RP2040/RP2350 GPIO pad control block.
pub struct RpPadsBank {
    name: String,
    state: Rc<RefCell<RpPadsState>>,
}

/// Machine-facing view of pad electrical configuration.
#[derive(Clone)]
pub struct RpPadsHandle {
    state: Rc<RefCell<RpPadsState>>,
}

impl RpPadsBank {
    /// Creates a pad bank and a machine-facing electrical-state handle.
    pub fn new(name: impl Into<String>, pins: u8, variant: RpPadsVariant) -> (Self, RpPadsHandle) {
        let reset = match variant {
            RpPadsVariant::Rp2040 => 0x56,
            RpPadsVariant::Rp2350 => 0x116,
        };
        let state = Rc::new(RefCell::new(RpPadsState {
            variant,
            voltage_select: 0,
            controls: vec![reset; usize::from(pins)],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RpPadsHandle { state },
        )
    }

    fn update(current: u32, alias: u64, value: u32) -> u32 {
        match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("two-bit RP atomic alias"),
        }
    }
}

impl RpPadsHandle {
    /// Returns the native pad control word for one GPIO.
    pub fn control(&self, pin: u8) -> Option<u32> {
        self.state.borrow().controls.get(usize::from(pin)).copied()
    }

    /// Returns the configured digital pull, if exactly one pull is enabled.
    pub fn pull(&self, pin: u8) -> Option<Logic> {
        let control = self.control(pin)?;
        match (control & (1 << 3) != 0, control & (1 << 2) != 0) {
            (true, false) => Some(Logic::One),
            (false, true) => Some(Logic::Zero),
            _ => None,
        }
    }

    /// Returns whether the pad output driver is disabled.
    pub fn output_disabled(&self, pin: u8) -> bool {
        self.control(pin)
            .is_none_or(|control| control & (1 << 7) != 0)
    }

    /// Returns whether the pad input buffer is enabled.
    pub fn input_enabled(&self, pin: u8) -> bool {
        self.control(pin)
            .is_some_and(|control| control & (1 << 6) != 0)
    }
}

impl Device for RpPadsBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new("RP pads do not support doubleword access"));
        }
        let register_offset = (offset & 0x0fff) & !3;
        let lane = offset & 3;
        let state = self.state.borrow();
        let value = if register_offset == 0 {
            state.voltage_select
        } else {
            let pin = usize::try_from(register_offset / 4 - 1).expect("pad index fits");
            *state
                .controls
                .get(pin)
                .ok_or_else(|| DeviceError::new("RP pad read outside bonded GPIOs"))?
        };
        Ok(match width {
            AccessWidth::Byte => u64::from(value >> (lane * 8) & 0xff),
            AccessWidth::HalfWord => u64::from(value >> ((lane & 2) * 8) & 0xffff),
            AccessWidth::Word => u64::from(value),
            AccessWidth::DoubleWord => unreachable!("checked above"),
        })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new("RP pads do not support doubleword access"));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = (offset & 0x0fff) & !3;
        let value = match width {
            AccessWidth::Byte => {
                let byte = value as u32 & 0xff;
                byte.wrapping_mul(0x0101_0101)
            }
            AccessWidth::HalfWord => {
                let half = value as u32 & 0xffff;
                half | (half << 16)
            }
            AccessWidth::Word => value as u32,
            AccessWidth::DoubleWord => unreachable!("checked above"),
        };
        let mut state = self.state.borrow_mut();
        if register_offset == 0 {
            state.voltage_select = Self::update(state.voltage_select, alias, value) & 1;
            return Ok(());
        }
        let pin = usize::try_from(register_offset / 4 - 1).expect("pad index fits");
        let mask = match state.variant {
            RpPadsVariant::Rp2040 => 0xff,
            RpPadsVariant::Rp2350 => 0x1ff,
        };
        let control = state
            .controls
            .get_mut(pin)
            .ok_or_else(|| DeviceError::new("RP pad write outside bonded GPIOs"))?;
        *control = Self::update(*control, alias, value) & mask;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.voltage_select = 0;
        let reset = match state.variant {
            RpPadsVariant::Rp2040 => 0x56,
            RpPadsVariant::Rp2350 => 0x116,
        };
        state.controls.fill(reset);
    }
}
