use super::*;

/// ESP32 GPIO matrix output/enable register slice.
///
/// The ESP32-S3 exposes GPIO pins 0 through 48 across low and high 32-bit
/// register banks. Other ESP targets can use the same device with only their
/// implemented low-bank pin count.
pub struct EspGpio {
    name: String,
    pins: u8,
    strap: u16,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl EspGpio {
    /// Creates the GPIO banks and an external-stimulus handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        Self::new_with_strap(name, pins, path, hub, 0)
    }

    /// Creates the GPIO banks with the value sampled into the read-only strap
    /// register at reset.
    pub fn new_with_strap(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
        strap: u16,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (state, signals, handle) = vendor_gpio_wide(pins, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                pins,
                strap,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn resolved_input(&self, high_bank: bool) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        let start = if high_bank { 32 } else { 0 };
        let end = if high_bank {
            self.pins.min(64)
        } else {
            self.pins.min(32)
        };
        (start..end).fold(0_u32, |value, pin| {
            if state.nets[usize::from(pin)].resolved() == Logic::One {
                value | (1_u32 << (pin - start))
            } else {
                value
            }
        })
    }
}

impl Device for EspGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("ESP GPIO requires word access"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = match offset {
            0x04 | 0x08 | 0x0c => state.output,
            0x10 | 0x14 | 0x18 => state.output_high,
            0x20 | 0x24 | 0x28 => state.direction,
            0x2c | 0x30 | 0x34 => state.direction_high,
            0x38 => u32::from(self.strap),
            0x3c => {
                drop(state);
                return Ok(u64::from(self.resolved_input(false)));
            }
            0x40 => {
                drop(state);
                return Ok(u64::from(self.resolved_input(true)));
            }
            _ => 0,
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
            return Err(DeviceError::new("ESP GPIO requires word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits u32");
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        match offset {
            0x04 => state.output = value,
            0x08 => state.output |= value,
            0x0c => state.output &= !value,
            0x10 => state.output_high = value,
            0x14 => state.output_high |= value,
            0x18 => state.output_high &= !value,
            0x20 => state.direction = value,
            0x24 => state.direction |= value,
            0x28 => state.direction &= !value,
            0x2c => state.direction_high = value,
            0x30 => state.direction_high |= value,
            0x34 => state.direction_high &= !value,
            _ => return Ok(()),
        }
        drop(state);
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }
}
