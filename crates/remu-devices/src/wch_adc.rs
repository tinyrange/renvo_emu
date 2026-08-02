use super::*;

const ADC_STATUS_EOC: u32 = 1 << 1;
const ADC_CONTROL1_EOCIE: u32 = 1 << 5;
const ADC_CONTROL2_ADON: u32 = 1;
const ADC_CONTROL2_SWSTART: u32 = 1 << 22;

/// Scheduler-facing handle for deterministic ADC channel stimuli and EOC state.
#[derive(Clone)]
pub struct WchAdcHandle {
    state: Rc<RefCell<WchAdcState>>,
}

impl WchAdcHandle {
    /// Sets a deterministic external or internal channel sample.
    pub fn set_channel_sample(&self, channel: u8, value: u16) {
        if let Some(sample) = self
            .state
            .borrow_mut()
            .samples
            .get_mut(usize::from(channel))
        {
            *sample = value & 0x03ff;
        }
    }

    /// Returns whether an end-of-conversion interrupt is pending.
    pub fn interrupt_pending(&self, now: SimTime) -> bool {
        let state = self.state.borrow();
        state.status & ADC_STATUS_EOC != 0
            && state.control1 & ADC_CONTROL1_EOCIE != 0
            && state.control2 & ADC_CONTROL2_ADON != 0
            && now.ticks() >= state.last_conversion
    }
}

struct WchAdcState {
    status: u32,
    control1: u32,
    control2: u32,
    sample_time1: u32,
    sample_time2: u32,
    sequence1: u32,
    sequence2: u32,
    sequence3: u32,
    data: u16,
    samples: [u16; 16],
    last_conversion: u64,
}

impl WchAdcState {
    fn reset() -> Self {
        Self {
            status: 0,
            control1: 0,
            control2: 0,
            sample_time1: 0,
            sample_time2: 0,
            sequence1: 0,
            sequence2: 0,
            sequence3: 0,
            data: 0,
            samples: [0; 16],
            last_conversion: 0,
        }
    }

    fn selected_channel(&self) -> usize {
        usize::try_from(self.sequence3 & 0x1f)
            .expect("WCH ADC channel selector fits usize")
            .min(self.samples.len() - 1)
    }

    fn start_conversion(&mut self, at: SimTime) {
        if self.control2 & ADC_CONTROL2_ADON == 0 {
            return;
        }
        self.data = self.samples[self.selected_channel()];
        self.status |= ADC_STATUS_EOC;
        self.last_conversion = at.ticks();
    }
}

/// Functional CH32V003/CH32V006 10-bit ADC1 register block.
///
/// A conversion is deterministic and completes when software sets ADON and
/// SWSTART. Host-side channel samples can be supplied through [`WchAdcHandle`]
/// and the model exposes EOC/DR and the regular-sequence registers used by
/// WCH HAL code. Analog settling, sample-clock timing, injected sequences, and
/// touch-key behavior remain outside this functional slice.
pub struct WchAdc {
    name: String,
    state: Rc<RefCell<WchAdcState>>,
}

impl WchAdc {
    /// Creates a reset ADC1 and its deterministic input handle.
    pub fn new(name: impl Into<String>) -> (Self, WchAdcHandle) {
        let state = Rc::new(RefCell::new(WchAdcState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchAdcHandle { state },
        )
    }

    fn require_word(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH ADC requires aligned word access"));
        }
        Ok(())
    }
}

impl Device for WchAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_word(offset, width)?;
        let mut state = self.state.borrow_mut();
        let value = match offset {
            0x00 => state.status,
            0x04 => state.control1,
            0x08 => state.control2,
            0x0c => state.sample_time1,
            0x10 => state.sample_time2,
            0x2c => state.sequence1,
            0x30 => state.sequence2,
            0x34 => state.sequence3,
            0x4c => {
                state.status &= !ADC_STATUS_EOC;
                u32::from(state.data)
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH ADC read at offset {offset:#x}"
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
        Self::require_word(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked WCH ADC register value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.status &= !value,
            0x04 => state.control1 = value & 0x00ff_ffff,
            0x08 => {
                state.control2 = value & 0x00ff_ffff;
                if value & ADC_CONTROL2_SWSTART != 0 {
                    state.start_conversion(at);
                    state.control2 &= !ADC_CONTROL2_SWSTART;
                }
            }
            0x0c => state.sample_time1 = value,
            0x10 => state.sample_time2 = value,
            0x2c => state.sequence1 = value,
            0x30 => state.sequence2 = value,
            0x34 => state.sequence3 = value,
            0x4c => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH ADC write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let samples = self.state.borrow().samples;
        let mut state = WchAdcState::reset();
        state.samples = samples;
        *self.state.borrow_mut() = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adc_sequence_selects_host_sample_and_clears_eoc_on_data_read() {
        let (mut adc, handle) = WchAdc::new("adc");
        handle.set_channel_sample(3, 0x2aa);
        adc.write(0x34, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        adc.write(
            0x08,
            AccessWidth::Word,
            u64::from(ADC_CONTROL2_ADON),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            0x08,
            AccessWidth::Word,
            u64::from(ADC_CONTROL2_ADON | ADC_CONTROL2_SWSTART),
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert_eq!(
            adc.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(ADC_STATUS_EOC),
            u64::from(ADC_STATUS_EOC)
        );
        assert_eq!(
            adc.read(0x4c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x2aa
        );
        assert_eq!(
            adc.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(ADC_STATUS_EOC),
            0
        );
    }

    #[test]
    fn adc_eoc_interrupt_requires_control_enable_and_power() {
        let (mut adc, handle) = WchAdc::new("adc");
        handle.set_channel_sample(0, 77);
        adc.write(
            0x04,
            AccessWidth::Word,
            u64::from(ADC_CONTROL1_EOCIE),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            0x08,
            AccessWidth::Word,
            u64::from(ADC_CONTROL2_SWSTART),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending(SimTime::ZERO));
        adc.write(
            0x08,
            AccessWidth::Word,
            u64::from(ADC_CONTROL2_ADON),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            0x08,
            AccessWidth::Word,
            u64::from(ADC_CONTROL2_ADON | ADC_CONTROL2_SWSTART),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.interrupt_pending(SimTime::ZERO));
    }
}
