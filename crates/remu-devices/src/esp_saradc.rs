use super::*;

const REGISTER_BYTES: usize = 0x400;
const CTRL: usize = 0x00;
const CTRL2: usize = 0x04;
const FSM_WAIT: usize = 0x0c;
const SAR1_STATUS: usize = 0x10;
const SAR2_STATUS: usize = 0x14;
const SAR1_PATT_BASE: usize = 0x18;
const SAR2_PATT_BASE: usize = 0x28;
const ADC1_DATA: usize = 0x40;
const THRES0_CTRL: usize = 0x44;
const THRES1_CTRL: usize = 0x48;
const THRES_CTRL: usize = 0x58;
const INT_ENA: usize = 0x5c;
const INT_RAW: usize = 0x60;
const INT_ST: usize = 0x64;
const INT_CLR: usize = 0x68;
const ADC2_DATA: usize = 0x78;
const DATE: usize = 0x3fc;

const SAMPLE_MASK: u32 = 0x1ffff;
const PATTERN_MASK: u32 = 0x00ff_ffff;
const DONE_MASK: u32 = 0xc000_0000;
const THRESHOLD_MASK: u32 = 0x3c00_0000;
const INTERRUPT_MASK: u32 = DONE_MASK | THRESHOLD_MASK;

/// Host-side observation and analogue-input handle for the ESP32-S3 SAR ADC.
#[derive(Clone)]
pub struct Esp32S3SarAdcHandle {
    state: Rc<RefCell<Esp32S3SarAdcState>>,
}

impl Esp32S3SarAdcHandle {
    /// Overrides the deterministic sample returned by one SAR unit.
    ///
    /// The value is masked to the native 17-bit data width. Clearing the
    /// override restores the functional channel/attenuation formula.
    pub fn set_input(&self, adc: usize, value: u32) -> Result<(), DeviceError> {
        let mut state = self.state.borrow_mut();
        let input = state
            .inputs
            .get_mut(adc)
            .ok_or_else(|| DeviceError::new(format!("SAR ADC unit {adc} is out of range")))?;
        *input = Some(value & SAMPLE_MASK);
        Ok(())
    }

    /// Restores the deterministic sample formula for one SAR unit.
    pub fn clear_input(&self, adc: usize) -> Result<(), DeviceError> {
        let mut state = self.state.borrow_mut();
        let input = state
            .inputs
            .get_mut(adc)
            .ok_or_else(|| DeviceError::new(format!("SAR ADC unit {adc} is out of range")))?;
        *input = None;
        Ok(())
    }

    /// Returns the most recently completed sample from one SAR unit.
    pub fn sample(&self, adc: usize) -> Result<u32, DeviceError> {
        self.state
            .borrow()
            .samples
            .get(adc)
            .copied()
            .ok_or_else(|| DeviceError::new(format!("SAR ADC unit {adc} is out of range")))
    }
}

struct Esp32S3SarAdcState {
    registers: Vec<u32>,
    samples: [u32; 2],
    inputs: [Option<u32>; 2],
    hub: SignalHub,
    signals: [SignalId; 2],
}

impl Esp32S3SarAdcState {
    fn register(&self, offset: usize) -> u32 {
        self.registers[offset / 4]
    }

    fn set_register(&mut self, offset: usize, value: u32) {
        self.registers[offset / 4] = value;
    }

    fn publish(&self, adc: usize, value: u32, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.signals[adc],
                SignalValue::from_u64(u64::from(value), 17)
                    .expect("fixed ESP32-S3 SAR ADC signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn pattern_item(&self, adc: usize) -> u8 {
        let base = if adc == 0 {
            SAR1_PATT_BASE
        } else {
            SAR2_PATT_BASE
        };
        // Each pattern-table register contains four 8-bit items. The native
        // pattern item uses bits 4:0 for channel and bits 7:6 for attenuation.
        let table = self.register(base);
        u8::try_from(table & 0xff).expect("pattern item is an 8-bit value")
    }

    fn fallback_sample(channel: u32, attenuation: u32) -> u32 {
        // This is deliberately a stable functional source, not an electrical
        // or calibrated analogue model. It keeps compiler/driver tests useful
        // while allowing callers to inject a board-specific value through the
        // observation handle.
        0x1000_u32
            .saturating_add(channel.saturating_mul(0x0200))
            .saturating_add(attenuation.saturating_mul(0x1000))
            .min(SAMPLE_MASK)
    }

    fn update_threshold_interrupts(&mut self, channel: u32, sample: u32) {
        let enabled = self.register(THRES_CTRL);
        let sample = sample & 0x1fff;
        for (index, register) in [THRES0_CTRL, THRES1_CTRL].into_iter().enumerate() {
            let enable_bit = 31 - index;
            if enabled & (1 << enable_bit) == 0 {
                continue;
            }
            let config = self.register(register);
            let configured_channel = config & 0x1f;
            if configured_channel != channel {
                continue;
            }
            let low = (config >> 18) & 0x1fff;
            let high = (config >> 5) & 0x1fff;
            if sample > high {
                self.registers[INT_RAW / 4] |= 1 << (29 - index * 2);
            }
            if sample < low {
                self.registers[INT_RAW / 4] |= 1 << (27 - index * 2);
            }
        }
    }

    fn refresh_interrupt_status(&mut self) {
        self.registers[INT_ST / 4] = self.register(INT_RAW) & self.register(INT_ENA);
    }

    fn sample(&mut self, adc: usize, at: SimTime) -> Result<(), DeviceError> {
        let item = self.pattern_item(adc);
        let channel = u32::from(item & 0x1f);
        let attenuation = u32::from((item >> 6) & 0x03);
        let value = self.inputs[adc].unwrap_or_else(|| Self::fallback_sample(channel, attenuation));
        self.samples[adc] = value;
        let (data, status, interrupt) = if adc == 0 {
            (ADC1_DATA, SAR1_STATUS, 1 << 31)
        } else {
            (ADC2_DATA, SAR2_STATUS, 1 << 30)
        };
        self.set_register(data, value);
        self.set_register(status, value);
        self.registers[INT_RAW / 4] |= interrupt;
        if adc == 0 {
            self.update_threshold_interrupts(channel, value);
        }
        self.refresh_interrupt_status();
        self.publish(adc, value, at)
    }

    fn trigger(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let ctrl = self.register(CTRL);
        let mode = (ctrl >> 3) & 0x03;
        match mode {
            1 => {
                self.sample(0, at)?;
                self.sample(1, at)?;
            }
            2 => {
                // Alternate mode advances the two SAR units on successive
                // triggers. A one-shot functional model completes both units
                // so software can observe a deterministic pair immediately.
                self.sample(0, at)?;
                self.sample(1, at)?;
            }
            _ if ctrl & (1 << 5) != 0 => self.sample(1, at)?,
            _ => self.sample(0, at)?,
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.samples = [0; 2];
        self.inputs = [None; 2];
        self.registers[CTRL / 4] = 4 << 7;
        self.registers[CTRL2 / 4] = 10 << 12;
        self.registers[FSM_WAIT / 4] = (0xff << 16) | (8 << 8) | 8;
        self.registers[THRES0_CTRL / 4] = (0x1fff << 5) | 13;
        self.registers[THRES1_CTRL / 4] = (0x1fff << 5) | 13;
        self.registers[DATE / 4] = 0x0210_1180;
    }
}

/// Functional ESP32-S3 APB SAR ADC register block.
///
/// This model follows the native register addresses and reset defaults from
/// Espressif's `apb_saradc_reg.h`. One-shot pattern-table conversions complete
/// synchronously at the current abstract simulation time. It intentionally does
/// not model analogue voltage, calibration, DMA, continuous clock timing, or
/// the separate SENS temperature-sensor block.
pub struct Esp32S3SarAdc {
    name: String,
    state: Rc<RefCell<Esp32S3SarAdcState>>,
}

impl Esp32S3SarAdc {
    /// Creates the native APB SAR ADC block and its host-input handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3SarAdcHandle), SignalError> {
        let signals = [
            hub.declare(
                "board.esp32s3.saradc.adc1",
                SignalValue::from_u64(0, 17)?,
                Some("ESP32-S3 SAR ADC1 sample".to_owned()),
            )?,
            hub.declare(
                "board.esp32s3.saradc.adc2",
                SignalValue::from_u64(0, 17)?,
                Some("ESP32-S3 SAR ADC2 sample".to_owned()),
            )?,
        ];
        let state = Rc::new(RefCell::new(Esp32S3SarAdcState {
            registers: vec![0; REGISTER_BYTES / 4],
            samples: [0; 2],
            inputs: [None; 2],
            hub,
            signals,
        }));
        state.borrow_mut().reset();
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3SarAdcHandle { state },
        ))
    }
}

impl Device for Esp32S3SarAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SAR ADC requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("SAR ADC offset fits usize");
        if offset >= REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        Ok(u64::from(self.state.borrow().register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SAR ADC requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("SAR ADC offset fits usize");
        if offset >= REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            CTRL => {
                // START and START_FORCE are command bits and self-clear after
                // the functional one-shot completes; all other documented
                // control fields remain software-visible.
                state.set_register(CTRL, value & !0x03);
                if value & 0x03 != 0 {
                    state.trigger(at)?;
                }
            }
            SAR1_STATUS | SAR2_STATUS | ADC1_DATA | ADC2_DATA | INT_RAW | INT_ST | DATE => {
                // Status, conversion data, and interrupt status are read-only;
                // DATE is retained as a reset-time identification constant.
                if offset == DATE {
                    state.set_register(DATE, value);
                }
            }
            SAR1_PATT_BASE..=0x24 | SAR2_PATT_BASE..=0x34 => {
                state.set_register(offset, value & PATTERN_MASK);
            }
            THRES0_CTRL | THRES1_CTRL => state.set_register(offset, value & 0x7fff_ffff),
            THRES_CTRL => state.set_register(THRES_CTRL, value & 0xf800_0000),
            INT_ENA => {
                state.set_register(INT_ENA, value & INTERRUPT_MASK);
                state.refresh_interrupt_status();
            }
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value & INTERRUPT_MASK);
                state.set_register(INT_CLR, 0);
                state.refresh_interrupt_status();
            }
            _ => state.set_register(offset, value),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_mode_pattern_conversion_sets_native_data_and_done_interrupt() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub.clone()).unwrap();
        adc.write(INT_ENA as u64, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        adc.write(
            SAR1_PATT_BASE as u64,
            AccessWidth::Word,
            (2 << 6) | 3,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(CTRL as u64, AccessWidth::Word, 1, SimTime::from_ticks(4))
            .unwrap();

        let expected = 0x1000 + (3 * 0x0200) + (2 * 0x1000);
        assert_eq!(handle.sample(0).unwrap(), expected);
        assert_eq!(
            adc.read(ADC1_DATA as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            expected as u64
        );
        assert_eq!(
            adc.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 31
        );
        let signal = hub.with_registry(|registry| registry.find("board.esp32s3.saradc.adc1"));
        assert!(signal.is_some());

        adc.write(INT_CLR as u64, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn double_mode_and_host_input_override_cover_both_sar_units() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub).unwrap();
        handle.set_input(1, 0x12345).unwrap();
        adc.write(CTRL as u64, AccessWidth::Word, (1 << 3) | 1, SimTime::ZERO)
            .unwrap();
        assert_ne!(
            adc.read(ADC1_DATA as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            adc.read(ADC2_DATA as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x12345
        );
        assert_eq!(
            adc.read(INT_RAW as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0xc000_0000
        );
    }

    #[test]
    fn threshold_monitor_sets_high_and_low_interrupts_for_matching_channel() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub).unwrap();
        handle.set_input(0, 0x100).unwrap();
        adc.write(
            THRES0_CTRL as u64,
            AccessWidth::Word,
            (0x200 << 18) | 3,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(THRES_CTRL as u64, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        adc.write(SAR1_PATT_BASE as u64, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        adc.write(CTRL as u64, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_ne!(
            adc.read(INT_RAW as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & (1 << 27),
            0
        );
    }
}
