use super::*;

/// ESP32 GPIO matrix output/enable register slice.
///
/// ESP32-S3 exposes 49 package GPIOs as a low 32-bit bank and a high bank;
/// the high-bank registers are modeled here as the native `OUT1`, `ENABLE1`,
/// and `IN1` windows.
pub struct EspGpio {
    name: String,
    pins: u8,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl EspGpio {
    /// Creates the low GPIO bank and an external-stimulus handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), SignalError> {
        let (state, signals, handle) = vendor_gpio(pins, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                pins,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn resolved_input(&self) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        (0..self.pins).fold(0_u32, |value, pin| {
            if state.nets[usize::from(pin)].resolved() == Logic::One {
                value | (1_u32 << pin)
            } else {
                value
            }
        })
    }

    fn resolved_input_high(&self) -> u32 {
        let state = self.state.lock().expect("GPIO lock poisoned");
        (32..self.pins).fold(0_u32, |value, pin| {
            if state.nets[usize::from(pin)].resolved() == Logic::One {
                value | (1_u32 << (pin - 32))
            } else {
                value
            }
        })
    }

    fn high_mask(&self) -> u32 {
        let count = self.pins.saturating_sub(32);
        if count == 0 {
            0
        } else if count >= 32 {
            u32::MAX
        } else {
            (1_u32 << count) - 1
        }
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
            0x10 | 0x14 | 0x18 => state.output_hi,
            0x20 | 0x24 | 0x28 => state.direction,
            0x2c | 0x30 | 0x34 => state.direction_hi,
            0x3c => {
                drop(state);
                return Ok(u64::from(self.resolved_input()));
            }
            0x40 => {
                drop(state);
                return Ok(u64::from(self.resolved_input_high()));
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
            0x10 => state.output_hi = value & self.high_mask(),
            0x14 => state.output_hi |= value,
            0x18 => state.output_hi &= !value,
            0x20 => state.direction = value,
            0x24 => state.direction |= value,
            0x28 => state.direction &= !value,
            0x2c => state.direction_hi = value & self.high_mask(),
            0x30 => state.direction_hi |= value,
            0x34 => state.direction_hi &= !value,
            _ => return Ok(()),
        }
        drop(state);
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }
}
