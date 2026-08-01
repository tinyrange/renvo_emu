use super::*;

/// Host-facing deterministic ADC input state.
#[derive(Clone)]
pub struct AdcHandle {
    samples: Arc<Mutex<[u16; 5]>>,
    conversions: Arc<Mutex<Vec<u16>>>,
}

impl AdcHandle {
    /// Sets the next deterministic value returned for an ADC channel.
    pub fn set_sample(&self, channel: usize, value: u16) -> bool {
        let mut samples = self.samples.lock().expect("ADC sample lock poisoned");
        let Some(sample) = samples.get_mut(channel) else {
            return false;
        };
        *sample = value & 0x0fff;
        true
    }

    /// Returns conversion results in execution order.
    pub fn conversions(&self) -> Vec<u16> {
        self.conversions
            .lock()
            .expect("ADC conversion lock poisoned")
            .clone()
    }

    fn sample(&self, channel: usize) -> u16 {
        let value = self
            .samples
            .lock()
            .expect("ADC sample lock poisoned")
            .get(channel)
            .copied()
            .unwrap_or(0);
        self.conversions
            .lock()
            .expect("ADC conversion lock poisoned")
            .push(value);
        value
    }

    fn clear(&self) {
        self.conversions
            .lock()
            .expect("ADC conversion lock poisoned")
            .clear();
    }
}

/// Deterministic RP2040/RP2350 SAR ADC and temperature-sensor slice.
///
/// Channel values are functional rather than analog: channel 0-3 default to mid-scale, channel
/// 4 represents the internal temperature-sensor input, and callers can override any channel via
/// [`AdcHandle::set_sample`]. A start command performs an immediate conversion and optionally
/// pushes it into the documented FIFO.
pub struct FunctionalAdc {
    name: String,
    cs: u32,
    result: u16,
    fcs: u32,
    div: u32,
    intr: u32,
    inte: u32,
    intf: u32,
    fifo: VecDeque<u16>,
    handle: AdcHandle,
}

impl FunctionalAdc {
    const CS: u64 = 0x00;
    const RESULT: u64 = 0x04;
    const FCS: u64 = 0x08;
    const FIFO: u64 = 0x0c;
    const DIV: u64 = 0x10;
    const INTR: u64 = 0x14;
    const INTE: u64 = 0x18;
    const INTF: u64 = 0x1c;
    const INTS: u64 = 0x20;
    const START_ONCE: u32 = 1 << 2;
    const START_MANY: u32 = 1 << 3;
    const READY: u32 = 1 << 8;
    const FIFO_ENABLE: u32 = 1;
    const FIFO_LEVEL_SHIFT: u32 = 16;

    /// Creates a reset ADC and host input handle.
    pub fn new(name: impl Into<String>) -> (Self, AdcHandle) {
        let handle = AdcHandle {
            samples: Arc::new(Mutex::new([2048; 5])),
            conversions: Arc::new(Mutex::new(Vec::new())),
        };
        (
            Self {
                name: name.into(),
                cs: 0,
                result: 0,
                fcs: 0,
                div: 0,
                intr: 0,
                inte: 0,
                intf: 0,
                fifo: VecDeque::new(),
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn check_access(offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ADC requires aligned word access"));
        }
        Ok(offset & 0x0fff)
    }

    fn convert(&mut self) {
        let channel = usize::try_from((self.cs >> 12) & 7).expect("ADC channel fits");
        self.result = self.handle.sample(channel.min(4));
        if self.fcs & Self::FIFO_ENABLE != 0 {
            self.fifo.push_back(self.result);
            let threshold = ((self.fcs >> 4) & 0xf) as usize;
            if threshold == 0 || self.fifo.len() >= threshold {
                self.intr |= 1;
            }
        }
    }

    fn fcs_value(&self) -> u32 {
        (self.fcs & 0xffff) | ((self.fifo.len() as u32) << Self::FIFO_LEVEL_SHIFT)
    }
}

impl Device for FunctionalAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let offset = Self::check_access(offset, width)?;
        let value = match offset {
            Self::CS => self.cs | Self::READY,
            Self::RESULT => u32::from(self.result),
            Self::FCS => self.fcs_value(),
            Self::FIFO => u32::from(self.fifo.pop_front().unwrap_or(0)),
            Self::DIV => self.div,
            Self::INTR => self.intr,
            Self::INTE => self.inte,
            Self::INTF => self.intf,
            Self::INTS => (self.intr & self.inte) | self.intf,
            _ => 0,
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
        let offset = Self::check_access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("ADC value fits");
        match offset {
            Self::CS => {
                self.cs = value & !(Self::READY);
                if value & (Self::START_ONCE | Self::START_MANY) != 0 {
                    self.convert();
                }
            }
            Self::FCS => self.fcs = value & 0x00ff_00ff,
            Self::DIV => self.div = value,
            Self::INTR => self.intr &= !value,
            Self::INTE => self.inte = value & 1,
            Self::INTF => self.intf = value & 1,
            Self::FIFO => {}
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.cs = 0;
        self.result = 0;
        self.fcs = 0;
        self.div = 0;
        self.intr = 0;
        self.inte = 0;
        self.intf = 0;
        self.fifo.clear();
        self.handle.clear();
    }
}
