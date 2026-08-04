use super::*;

pub(crate) struct GpioState {
    pub(crate) direction: u32,
    pub(crate) output: u32,
    pub(crate) direction_high: u32,
    pub(crate) output_high: u32,
    pub(crate) nets: Vec<DigitalNet>,
}

/// Host-facing GPIO input and state control.
#[derive(Clone)]
pub struct GpioHandle {
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl GpioHandle {
    /// Number of pins exposed by this port.
    pub fn pin_count(&self) -> usize {
        self.signals.len()
    }

    /// Drives or releases one external pin source.
    pub fn set_input(&self, pin: u8, value: Logic, at: SimTime) -> Result<(), DeviceError> {
        let index = usize::from(pin);
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        let net = state
            .nets
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("GPIO pin {pin} is out of range")))?;
        let update = net.drive(DriverId(1), value);
        drop(state);
        self.hub
            .set(
                self.signals[index],
                SignalValue::repeat(update.value, 1)
                    .expect("one-bit signal construction cannot fail"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    /// Current direction bit mask.
    pub fn direction(&self) -> u32 {
        self.state.lock().expect("GPIO lock poisoned").direction
    }

    /// Current output latch.
    pub fn output(&self) -> u32 {
        self.state.lock().expect("GPIO lock poisoned").output
    }

    /// Returns the currently resolved digital value for one pin.
    pub fn resolved(&self, pin: u8) -> Result<Logic, DeviceError> {
        self.state
            .lock()
            .expect("GPIO lock poisoned")
            .nets
            .get(usize::from(pin))
            .map(DigitalNet::resolved)
            .ok_or_else(|| DeviceError::new(format!("GPIO pin {pin} is out of range")))
    }

    pub(crate) fn drive_peripheral(
        &self,
        pin: u8,
        driver: u16,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let index = usize::from(pin);
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        let net = state
            .nets
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("GPIO pin {pin} is out of range")))?;
        let update = net.drive(DriverId(u32::from(driver)), value);
        drop(state);
        self.hub
            .set(
                self.signals[index],
                SignalValue::repeat(update.value, 1)
                    .expect("one-bit signal construction cannot fail"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }
}

/// Simple GPIO register facade with direction, output, and input registers.
pub struct FunctionalGpio {
    name: String,
    pins: u8,
    direction_offset: u64,
    output_offset: u64,
    input_offset: u64,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl FunctionalGpio {
    /// Creates a GPIO block and host input handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
        direction_offset: u64,
        output_offset: u64,
        input_offset: u64,
    ) -> Result<(Self, GpioHandle), SignalError> {
        if pins == 0 || pins > 32 {
            return Err(SignalError::WidthMismatch {
                expected: 1,
                actual: u16::from(pins),
            });
        }
        let mut signals = Vec::with_capacity(usize::from(pins));
        for pin in 0..pins {
            signals.push(hub.declare(
                format!("{path}.pin{pin}"),
                SignalValue::repeat(Logic::Z, 1)?,
                Some(format!("GPIO pin {pin}")),
            )?);
        }
        let state = Arc::new(Mutex::new(GpioState {
            direction: 0,
            output: 0,
            direction_high: 0,
            output_high: 0,
            nets: (0..pins).map(|_| DigitalNet::new()).collect(),
        }));
        let handle = GpioHandle {
            state: state.clone(),
            signals: signals.clone(),
            hub: hub.clone(),
        };
        Ok((
            Self {
                name: name.into(),
                pins,
                direction_offset,
                output_offset,
                input_offset,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn mask(&self) -> u32 {
        if self.pins == 32 {
            u32::MAX
        } else {
            (1_u32 << self.pins) - 1
        }
    }

    fn refresh_outputs(&self, at: SimTime) -> Result<(), DeviceError> {
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }
}

pub(crate) fn refresh_gpio(
    shared: &Arc<Mutex<GpioState>>,
    signals: &[SignalId],
    hub: &SignalHub,
    pins: u8,
    at: SimTime,
) -> Result<(), DeviceError> {
    let mut state = shared.lock().expect("GPIO lock poisoned");
    for pin in 0..pins {
        let bit = if pin < 32 {
            1_u32 << pin
        } else {
            1_u32 << (pin - 32)
        };
        let (direction, output) = if pin < 32 {
            (state.direction, state.output)
        } else {
            (state.direction_high, state.output_high)
        };
        let logic = if direction & bit == 0 {
            Logic::Z
        } else if output & bit == 0 {
            Logic::Zero
        } else {
            Logic::One
        };
        let update = state.nets[usize::from(pin)].drive(DriverId(0), logic);
        hub.set(
            signals[usize::from(pin)],
            SignalValue::repeat(update.value, 1).expect("one-bit signal construction cannot fail"),
            at,
        )
        .map_err(|error| DeviceError::new(error.to_string()))?;
    }
    Ok(())
}

type VendorGpioParts = (Arc<Mutex<GpioState>>, Vec<SignalId>, GpioHandle);

fn vendor_gpio_with_limit(
    pins: u8,
    path: &str,
    hub: &SignalHub,
    max_pins: u8,
) -> Result<VendorGpioParts, SignalError> {
    if pins == 0 || pins > max_pins {
        return Err(SignalError::WidthMismatch {
            expected: u16::from(max_pins),
            actual: u16::from(pins),
        });
    }
    let mut signals = Vec::with_capacity(usize::from(pins));
    for pin in 0..pins {
        signals.push(hub.declare(
            format!("{path}.pin{pin}"),
            SignalValue::repeat(Logic::Z, 1)?,
            Some(format!("GPIO pin {pin}")),
        )?);
    }
    let state = Arc::new(Mutex::new(GpioState {
        direction: 0,
        output: 0,
        direction_high: 0,
        output_high: 0,
        nets: (0..pins).map(|_| DigitalNet::new()).collect(),
    }));
    let handle = GpioHandle {
        state: state.clone(),
        signals: signals.clone(),
        hub: hub.clone(),
    };
    Ok((state, signals, handle))
}

pub(crate) fn vendor_gpio(
    pins: u8,
    path: &str,
    hub: &SignalHub,
) -> Result<VendorGpioParts, SignalError> {
    vendor_gpio_with_limit(pins, path, hub, 32)
}

pub(crate) fn vendor_gpio_wide(
    pins: u8,
    path: &str,
    hub: &SignalHub,
) -> Result<VendorGpioParts, SignalError> {
    vendor_gpio_with_limit(pins, path, hub, 64)
}

impl Device for FunctionalGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("GPIO facade requires word accesses"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = if offset == self.direction_offset {
            state.direction
        } else if offset == self.output_offset {
            state.output
        } else if offset == self.input_offset {
            let mut resolved = 0_u32;
            for pin in 0..self.pins {
                if state.nets[usize::from(pin)].resolved() == Logic::One {
                    resolved |= 1_u32 << pin;
                }
            }
            resolved
        } else {
            return Err(DeviceError::new(format!(
                "unmodeled GPIO read at offset {offset:#x}"
            )));
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
            return Err(DeviceError::new("GPIO facade requires word accesses"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked value always fits in u32")
            & self.mask();
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            if offset == self.direction_offset {
                state.direction = value;
            } else if offset == self.output_offset {
                state.output = value;
            } else {
                return Err(DeviceError::new(format!(
                    "unmodeled GPIO write at offset {offset:#x}"
                )));
            }
        }
        self.refresh_outputs(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            state.direction = 0;
            state.output = 0;
            for net in &mut state.nets {
                net.disconnect(DriverId(0));
            }
        }
        let _ = self.refresh_outputs(SimTime::ZERO);
    }
}

/// WCH `CH32V00x` GPIO register slice (`CFGLR/INDR/OUTDR/BSHR/BCR`).
pub struct WchGpio {
    name: String,
    pins: u8,
    config_low: u32,
    config_high: u32,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl WchGpio {
    /// Creates one WCH GPIO port and an external-stimulus handle.
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
                config_low: 0x4444_4444,
                config_high: 0x4444_4444,
                state,
                signals,
                hub,
            },
            handle,
        ))
    }

    fn update_direction(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let mut direction = 0_u32;
        for pin in 0..self.pins {
            let config = if pin < 8 {
                self.config_low >> (u32::from(pin) * 4)
            } else {
                self.config_high >> (u32::from(pin - 8) * 4)
            };
            if config & 3 != 0 {
                direction |= 1_u32 << pin;
            }
        }
        self.state.lock().expect("GPIO lock poisoned").direction = direction;
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
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
}

impl Device for WchGpio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("WCH GPIO requires word access"));
        }
        let state = self.state.lock().expect("GPIO lock poisoned");
        let value = match offset {
            0x00 => self.config_low,
            0x04 => self.config_high,
            0x08 => {
                drop(state);
                return Ok(u64::from(self.resolved_input()));
            }
            0x0c => state.output,
            0x10 | 0x14 | 0x18 => 0,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH GPIO read at offset {offset:#x}"
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
            return Err(DeviceError::new("WCH GPIO requires word access"));
        }
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits u32");
        let register_offset = offset & 0x0fff;
        match register_offset {
            0x00 => {
                self.config_low = value;
                return self.update_direction(at);
            }
            0x04 => {
                self.config_high = value;
                return self.update_direction(at);
            }
            0x0c => self.state.lock().expect("GPIO lock poisoned").output = value,
            0x10 => {
                let mut state = self.state.lock().expect("GPIO lock poisoned");
                state.output |= value & 0xffff;
                state.output &= !(value >> 16);
            }
            0x14 => self.state.lock().expect("GPIO lock poisoned").output &= !value,
            0x18 => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH GPIO write at offset {offset:#x}"
                )));
            }
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.config_low = 0x4444_4444;
        self.config_high = 0x4444_4444;
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}
