use super::*;

#[derive(Default)]
struct DacState {
    data: u16,
    control: u8,
    format: u8,
    sync: u8,
    vref: u8,
}

/// Host-facing RA4M1 DAC12 output state.
#[derive(Clone)]
pub struct RaDacHandle(Arc<Mutex<DacState>>);

impl RaDacHandle {
    /// Returns the right-aligned 12-bit output sample.
    pub fn value(&self) -> u16 {
        let state = self.0.lock().expect("RA DAC lock poisoned");
        if state.format & (1 << 7) != 0 {
            (state.data >> 4) & 0x0fff
        } else {
            state.data & 0x0fff
        }
    }

    /// Returns whether DAC channel 0 output is enabled.
    pub fn enabled(&self) -> bool {
        self.0.lock().expect("RA DAC lock poisoned").control & (1 << 6) != 0
    }
}

/// Functional RA4M1 DAC12 channel 0 and its waveform signal.
pub struct RaDac {
    name: String,
    state: Arc<Mutex<DacState>>,
    hub: SignalHub,
    output: SignalId,
}

impl RaDac {
    /// Creates DAC12 channel 0 and a 12-bit output signal.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, RaDacHandle), remu_signals::SignalError> {
        let output = hub.declare(
            path,
            SignalValue::from_u64(0, 12)?,
            Some("RA4M1 DAC12 channel 0 output sample".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(DacState {
            control: 0x1f,
            ..DacState::default()
        }));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                output,
            },
            RaDacHandle(state),
        ))
    }

    fn sample(&self) -> u16 {
        let state = self.state.lock().expect("RA DAC lock poisoned");
        if state.control & (1 << 6) == 0 {
            return 0;
        }
        if state.format & (1 << 7) != 0 {
            (state.data >> 4) & 0x0fff
        } else {
            state.data & 0x0fff
        }
    }

    fn refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.output,
                SignalValue::from_u64(u64::from(self.sample()), 12)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }
}

impl Device for RaDac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let value = match offset {
            0x00 if matches!(width, AccessWidth::HalfWord | AccessWidth::Word) => {
                u64::from(self.state.lock().expect("RA DAC lock poisoned").data)
            }
            0x00 => u64::from(self.state.lock().expect("RA DAC lock poisoned").data & 0xff),
            0x01 => u64::from(self.state.lock().expect("RA DAC lock poisoned").data >> 8),
            0x04 => u64::from(self.state.lock().expect("RA DAC lock poisoned").control),
            0x05 => u64::from(self.state.lock().expect("RA DAC lock poisoned").format),
            0x06 => u64::from(self.state.lock().expect("RA DAC lock poisoned").sync),
            0x07 => u64::from(self.state.lock().expect("RA DAC lock poisoned").vref),
            _ => 0,
        };
        Ok(value & width.value_mask())
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RA DAC lock poisoned");
        match offset {
            0x00 if matches!(width, AccessWidth::HalfWord | AccessWidth::Word) => {
                state.data = value as u16
            }
            0x00 => state.data = (state.data & 0xff00) | (value as u16 & 0xff),
            0x01 => state.data = (state.data & 0x00ff) | ((value as u16 & 0xff) << 8),
            // DACR has read-as-one reserved bits 4:0 and read-as-zero
            // reserved bits 7 and 5. Only DAOE0 is functional here.
            0x04 => state.control = 0x1f | (value as u8 & (1 << 6)),
            0x05 => state.format = value as u8 & (1 << 7),
            0x06 => state.sync = value as u8 & (1 << 7),
            0x07 => state.vref = value as u8 & 0x07,
            _ => return Ok(()),
        }
        drop(state);
        self.refresh(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA DAC lock poisoned") = DacState {
            control: 0x1f,
            ..DacState::default()
        };
        let _ = self.refresh(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dac12_latches_samples_and_applies_justification() {
        let hub = SignalHub::new();
        let (mut dac, handle) = RaDac::new("dac12", "board.ra.dac0", hub).unwrap();
        dac.write(0x00, AccessWidth::HalfWord, 0x0abc, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0x0abc);
        assert!(!handle.enabled());
        dac.write(0x04, AccessWidth::Byte, 1 << 6, SimTime::from_ticks(1))
            .unwrap();
        assert!(handle.enabled());
        dac.write(0x05, AccessWidth::Byte, 1 << 7, SimTime::from_ticks(2))
            .unwrap();
        dac.write(0x00, AccessWidth::HalfWord, 0xabc0, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(handle.value(), 0x0abc);
        assert_eq!(
            dac.read(0x04, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x5f
        );
        dac.write(0x05, AccessWidth::Byte, 0xff, SimTime::from_ticks(4))
            .unwrap();
        dac.write(0x06, AccessWidth::Byte, 0xff, SimTime::from_ticks(5))
            .unwrap();
        dac.write(0x07, AccessWidth::Byte, 0xff, SimTime::from_ticks(6))
            .unwrap();
        assert_eq!(
            dac.read(0x05, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x80
        );
        assert_eq!(
            dac.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x80
        );
        assert_eq!(
            dac.read(0x07, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x07
        );
    }
}
