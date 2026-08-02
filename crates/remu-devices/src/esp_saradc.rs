use super::*;

const REGISTER_BYTES: usize = 0x400;
const SAMPLE_MASK: u32 = 0x1ffff;
const INTERRUPT_MASK: u32 = 0xfc00_0000;

const CTRL_START: u32 = 1 << 1;
const CTRL_START_FORCE: u32 = 1;
const CTRL_SAR_SELECT: u32 = 1 << 5;
const CTRL_WORK_MODE_MASK: u32 = 0x18;
const CTRL_PATTERN_CLEAR_MASK: u32 = (1 << 23) | (1 << 24);
const CTRL_WRITE_MASK: u32 = 0xdfff_fffb;
const CTRL2_WRITE_MASK: u32 = 0x01ff_ffff;
const CTRL2_SAR1_INVERT: u32 = 1 << 9;
const CTRL2_SAR2_INVERT: u32 = 1 << 10;

/// ESP32-S3 APB SAR ADC register identifiers.
///
/// Offsets and access masks are derived from Espressif's
/// `apb_saradc_reg.h`.  Reserved offsets are intentionally not accepted by
/// the device model.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32s3SarAdcRegister {
    Ctrl,
    Ctrl2,
    FilterCtrl1,
    FsmWait,
    Sar1Status,
    Sar2Status,
    Sar1Pattern1,
    Sar1Pattern2,
    Sar1Pattern3,
    Sar1Pattern4,
    Sar2Pattern1,
    Sar2Pattern2,
    Sar2Pattern3,
    Sar2Pattern4,
    ArbiterCtrl,
    FilterCtrl0,
    Adc1Data,
    Threshold0Ctrl,
    Threshold1Ctrl,
    ThresholdCtrl,
    IntEnable,
    IntRaw,
    IntStatus,
    IntClear,
    DmaConfig,
    ClockConfig,
    Adc2Data,
    Date,
}

impl Esp32s3SarAdcRegister {
    /// Returns the native byte offset within the APB SAR ADC block.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Ctrl => 0x00,
            Self::Ctrl2 => 0x04,
            Self::FilterCtrl1 => 0x08,
            Self::FsmWait => 0x0c,
            Self::Sar1Status => 0x10,
            Self::Sar2Status => 0x14,
            Self::Sar1Pattern1 => 0x18,
            Self::Sar1Pattern2 => 0x1c,
            Self::Sar1Pattern3 => 0x20,
            Self::Sar1Pattern4 => 0x24,
            Self::Sar2Pattern1 => 0x28,
            Self::Sar2Pattern2 => 0x2c,
            Self::Sar2Pattern3 => 0x30,
            Self::Sar2Pattern4 => 0x34,
            Self::ArbiterCtrl => 0x38,
            Self::FilterCtrl0 => 0x3c,
            Self::Adc1Data => 0x40,
            Self::Threshold0Ctrl => 0x44,
            Self::Threshold1Ctrl => 0x48,
            Self::ThresholdCtrl => 0x58,
            Self::IntEnable => 0x5c,
            Self::IntRaw => 0x60,
            Self::IntStatus => 0x64,
            Self::IntClear => 0x68,
            Self::DmaConfig => 0x6c,
            Self::ClockConfig => 0x70,
            Self::Adc2Data => 0x78,
            Self::Date => 0x3fc,
        }
    }

    /// Resolves a native aligned byte offset to a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Ctrl),
            0x04 => Some(Self::Ctrl2),
            0x08 => Some(Self::FilterCtrl1),
            0x0c => Some(Self::FsmWait),
            0x10 => Some(Self::Sar1Status),
            0x14 => Some(Self::Sar2Status),
            0x18 => Some(Self::Sar1Pattern1),
            0x1c => Some(Self::Sar1Pattern2),
            0x20 => Some(Self::Sar1Pattern3),
            0x24 => Some(Self::Sar1Pattern4),
            0x28 => Some(Self::Sar2Pattern1),
            0x2c => Some(Self::Sar2Pattern2),
            0x30 => Some(Self::Sar2Pattern3),
            0x34 => Some(Self::Sar2Pattern4),
            0x38 => Some(Self::ArbiterCtrl),
            0x3c => Some(Self::FilterCtrl0),
            0x40 => Some(Self::Adc1Data),
            0x44 => Some(Self::Threshold0Ctrl),
            0x48 => Some(Self::Threshold1Ctrl),
            0x58 => Some(Self::ThresholdCtrl),
            0x5c => Some(Self::IntEnable),
            0x60 => Some(Self::IntRaw),
            0x64 => Some(Self::IntStatus),
            0x68 => Some(Self::IntClear),
            0x6c => Some(Self::DmaConfig),
            0x70 => Some(Self::ClockConfig),
            0x78 => Some(Self::Adc2Data),
            0x3fc => Some(Self::Date),
            _ => None,
        }
    }

    const fn read_mask(self) -> u32 {
        match self {
            Self::Ctrl => CTRL_WRITE_MASK,
            Self::Ctrl2 => CTRL2_WRITE_MASK,
            Self::FilterCtrl1 => 0xfc00_0000,
            Self::FsmWait => 0x00ff_ffff,
            Self::Sar1Status | Self::Sar2Status => u32::MAX,
            Self::Sar1Pattern1
            | Self::Sar1Pattern2
            | Self::Sar1Pattern3
            | Self::Sar1Pattern4
            | Self::Sar2Pattern1
            | Self::Sar2Pattern2
            | Self::Sar2Pattern3
            | Self::Sar2Pattern4 => 0x00ff_ffff,
            Self::ArbiterCtrl => 0x0000_1ffc,
            Self::FilterCtrl0 => 0x80ff_c000,
            Self::Adc1Data | Self::Adc2Data => SAMPLE_MASK,
            Self::Threshold0Ctrl | Self::Threshold1Ctrl => 0x7fff_ffff,
            Self::ThresholdCtrl => 0xf800_0000,
            Self::IntEnable | Self::IntRaw | Self::IntStatus | Self::IntClear => INTERRUPT_MASK,
            Self::DmaConfig => 0xc000_ffff,
            Self::ClockConfig => 0x007f_ffff,
            Self::Date => u32::MAX,
        }
    }

    const fn write_mask(self) -> u32 {
        match self {
            Self::Ctrl => CTRL_WRITE_MASK,
            Self::Ctrl2 => CTRL2_WRITE_MASK,
            Self::FilterCtrl1 => 0xfc00_0000,
            Self::FsmWait => 0x00ff_ffff,
            Self::Sar1Pattern1
            | Self::Sar1Pattern2
            | Self::Sar1Pattern3
            | Self::Sar1Pattern4
            | Self::Sar2Pattern1
            | Self::Sar2Pattern2
            | Self::Sar2Pattern3
            | Self::Sar2Pattern4 => 0x00ff_ffff,
            Self::ArbiterCtrl => 0x0000_1ffc,
            Self::FilterCtrl0 => 0x80ff_c000,
            Self::Threshold0Ctrl | Self::Threshold1Ctrl => 0x7fff_ffff,
            Self::ThresholdCtrl => 0xf800_0000,
            Self::IntEnable => INTERRUPT_MASK,
            Self::IntClear => INTERRUPT_MASK,
            Self::DmaConfig => 0xc000_ffff,
            Self::ClockConfig => 0x007f_ffff,
            Self::Date => u32::MAX,
            Self::Sar1Status
            | Self::Sar2Status
            | Self::Adc1Data
            | Self::Adc2Data
            | Self::IntRaw
            | Self::IntStatus => 0,
        }
    }
}

fn index(register: Esp32s3SarAdcRegister) -> usize {
    (register.offset() / 4) as usize
}

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
    alternate_next: usize,
    hub: SignalHub,
    signals: [SignalId; 2],
}

impl Esp32S3SarAdcState {
    fn register(&self, register: Esp32s3SarAdcRegister) -> u32 {
        self.registers[index(register)]
    }

    fn set_register(&mut self, register: Esp32s3SarAdcRegister, value: u32) {
        self.registers[index(register)] = value & register.read_mask();
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
        let register = if adc == 0 {
            Esp32s3SarAdcRegister::Sar1Pattern1
        } else {
            Esp32s3SarAdcRegister::Sar2Pattern1
        };
        // Each pattern-table register contains four 8-bit items. The native
        // pattern item uses bits 4:0 for channel and bits 7:6 for attenuation.
        let table = self.register(register);
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
        let enabled = self.register(Esp32s3SarAdcRegister::ThresholdCtrl);
        let sample = sample & 0x1fff;
        for (slot, register) in [
            Esp32s3SarAdcRegister::Threshold0Ctrl,
            Esp32s3SarAdcRegister::Threshold1Ctrl,
        ]
        .into_iter()
        .enumerate()
        {
            let enable_bit = 31 - slot;
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
                self.registers[index(Esp32s3SarAdcRegister::IntRaw)] |= 1 << (29 - slot * 2);
            }
            if sample < low {
                self.registers[index(Esp32s3SarAdcRegister::IntRaw)] |= 1 << (27 - slot * 2);
            }
        }
    }

    fn refresh_interrupt_status(&mut self) {
        self.registers[index(Esp32s3SarAdcRegister::IntStatus)] = self
            .register(Esp32s3SarAdcRegister::IntRaw)
            & self.register(Esp32s3SarAdcRegister::IntEnable);
    }

    fn sample(&mut self, adc: usize, at: SimTime) -> Result<(), DeviceError> {
        let item = self.pattern_item(adc);
        let channel = u32::from(item & 0x1f);
        let attenuation = u32::from((item >> 6) & 0x03);
        let mut value =
            self.inputs[adc].unwrap_or_else(|| Self::fallback_sample(channel, attenuation));
        let invert = if adc == 0 {
            CTRL2_SAR1_INVERT
        } else {
            CTRL2_SAR2_INVERT
        };
        if self.register(Esp32s3SarAdcRegister::Ctrl2) & invert != 0 {
            value = (!value) & SAMPLE_MASK;
        }
        self.samples[adc] = value;
        let (data, status, interrupt) = if adc == 0 {
            (
                Esp32s3SarAdcRegister::Adc1Data,
                Esp32s3SarAdcRegister::Sar1Status,
                1 << 31,
            )
        } else {
            (
                Esp32s3SarAdcRegister::Adc2Data,
                Esp32s3SarAdcRegister::Sar2Status,
                1 << 30,
            )
        };
        self.set_register(data, value);
        self.set_register(status, value);
        self.registers[index(Esp32s3SarAdcRegister::IntRaw)] |= interrupt;
        if adc == 0 {
            self.update_threshold_interrupts(channel, value);
        }
        self.refresh_interrupt_status();
        self.publish(adc, value, at)
    }

    fn trigger(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let ctrl = self.register(Esp32s3SarAdcRegister::Ctrl);
        let mode = (ctrl & CTRL_WORK_MODE_MASK) >> 3;
        match mode {
            1 => {
                self.sample(0, at)?;
                self.sample(1, at)?;
            }
            2 => {
                // Alternate mode selects one SAR unit per trigger.
                let adc = self.alternate_next;
                self.sample(adc, at)?;
                self.alternate_next ^= 1;
            }
            _ if ctrl & CTRL_SAR_SELECT != 0 => self.sample(1, at)?,
            _ => self.sample(0, at)?,
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.samples = [0; 2];
        self.inputs = [None; 2];
        self.alternate_next = 0;
        self.set_register(
            Esp32s3SarAdcRegister::Ctrl,
            (1 << 30) | (15 << 19) | (15 << 15) | (4 << 7) | (1 << 6),
        );
        self.set_register(Esp32s3SarAdcRegister::Ctrl2, (10 << 12) | (0xff << 1));
        self.set_register(Esp32s3SarAdcRegister::FsmWait, (0xff << 16) | (8 << 8) | 8);
        self.set_register(Esp32s3SarAdcRegister::Threshold0Ctrl, (0x1fff << 5) | 13);
        self.set_register(Esp32s3SarAdcRegister::Threshold1Ctrl, (0x1fff << 5) | 13);
        self.set_register(Esp32s3SarAdcRegister::ArbiterCtrl, (2 << 10) | (1 << 8));
        self.set_register(Esp32s3SarAdcRegister::FilterCtrl0, (13 << 19) | (13 << 14));
        self.set_register(Esp32s3SarAdcRegister::DmaConfig, 0xff);
        self.set_register(Esp32s3SarAdcRegister::ClockConfig, 4);
        self.set_register(Esp32s3SarAdcRegister::Date, 0x0210_1180);
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
            alternate_next: 0,
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
        let register = Esp32s3SarAdcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read at unsupported SAR ADC offset {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.borrow_mut();
        if register == Esp32s3SarAdcRegister::IntStatus {
            state.refresh_interrupt_status();
        }
        Ok(u64::from(state.register(register) & register.read_mask()))
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
        if value > u64::from(u32::MAX) {
            return Err(DeviceError::new(
                "ESP32-S3 SAR ADC rejects values wider than 32 bits",
            ));
        }
        let register = Esp32s3SarAdcRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at unsupported SAR ADC offset {offset:#x}",
                self.name
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "{} write to read-only SAR ADC register {register:?}",
                self.name
            )));
        }
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        match register {
            Esp32s3SarAdcRegister::Ctrl => {
                // START/START_FORCE and pattern-pointer clear commands are
                // strobes. Other documented control fields remain visible.
                state.set_register(
                    register,
                    value & write_mask & !(CTRL_START | CTRL_START_FORCE | CTRL_PATTERN_CLEAR_MASK),
                );
                if value & (CTRL_START | CTRL_START_FORCE) != 0 {
                    state.trigger(at)?;
                }
            }
            Esp32s3SarAdcRegister::IntEnable => {
                state.set_register(register, value & write_mask);
                state.refresh_interrupt_status();
            }
            Esp32s3SarAdcRegister::IntClear => {
                state.registers[index(Esp32s3SarAdcRegister::IntRaw)] &= !(value & write_mask);
                state.set_register(register, 0);
                state.refresh_interrupt_status();
            }
            _ => {
                let current = state.register(register);
                state.set_register(register, (current & !write_mask) | (value & write_mask));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.reset();
        let zero = SignalValue::from_u64(0, 17).expect("17-bit signal");
        let _ = state.hub.set(state.signals[0], zero.clone(), SimTime::ZERO);
        let _ = state.hub.set(state.signals[1], zero, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_mode_pattern_conversion_sets_native_data_and_done_interrupt() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub.clone()).unwrap();
        adc.write(
            Esp32s3SarAdcRegister::IntEnable.offset(),
            AccessWidth::Word,
            1 << 31,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Sar1Pattern1.offset(),
            AccessWidth::Word,
            (2 << 6) | 3,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Ctrl.offset(),
            AccessWidth::Word,
            1,
            SimTime::from_ticks(4),
        )
        .unwrap();

        let expected = 0x1000 + (3 * 0x0200) + (2 * 0x1000);
        assert_eq!(handle.sample(0).unwrap(), expected);
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Adc1Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            expected as u64
        );
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::IntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            1 << 31
        );
        let signal = hub.with_registry(|registry| registry.find("board.esp32s3.saradc.adc1"));
        assert!(signal.is_some());

        adc.write(
            Esp32s3SarAdcRegister::IntClear.offset(),
            AccessWidth::Word,
            1 << 31,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::IntStatus.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn double_mode_and_host_input_override_cover_both_sar_units() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub).unwrap();
        handle.set_input(1, 0x12345).unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Ctrl.offset(),
            AccessWidth::Word,
            (1 << 3) | 1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_ne!(
            adc.read(
                Esp32s3SarAdcRegister::Adc1Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Adc2Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x12345
        );
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::IntRaw.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
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
            Esp32s3SarAdcRegister::Threshold0Ctrl.offset(),
            AccessWidth::Word,
            (0x200 << 18) | 3,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::ThresholdCtrl.offset(),
            AccessWidth::Word,
            1 << 31,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Sar1Pattern1.offset(),
            AccessWidth::Word,
            3,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Ctrl.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_ne!(
            adc.read(
                Esp32s3SarAdcRegister::IntRaw.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
                & (1 << 27),
            0
        );
    }

    #[test]
    fn alternate_mode_advances_one_sar_and_ctrl2_inverts_samples() {
        let hub = SignalHub::new();
        let (mut adc, handle) = Esp32S3SarAdc::new("saradc", hub).unwrap();
        handle.set_input(0, 0x00100).unwrap();
        handle.set_input(1, 0x00200).unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Ctrl2.offset(),
            AccessWidth::Word,
            u64::from(CTRL2_SAR1_INVERT),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            Esp32s3SarAdcRegister::Ctrl.offset(),
            AccessWidth::Word,
            u64::from((2 << 3) | CTRL_START_FORCE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Adc1Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            u64::from((!0x00100_u32) & SAMPLE_MASK)
        );
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Adc2Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
        adc.write(
            Esp32s3SarAdcRegister::Ctrl.offset(),
            AccessWidth::Word,
            u64::from((2 << 3) | CTRL_START_FORCE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Adc2Data.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x200
        );
    }

    #[test]
    fn register_enum_applies_official_masks_and_rejects_reserved_accesses() {
        assert_eq!(
            Esp32s3SarAdcRegister::from_offset(0x70),
            Some(Esp32s3SarAdcRegister::ClockConfig)
        );
        assert_eq!(Esp32s3SarAdcRegister::from_offset(0x74), None);
        let hub = SignalHub::new();
        let (mut adc, _) = Esp32S3SarAdc::new("saradc", hub).unwrap();
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::Ctrl.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0x4000_0000 | (15 << 19) | (15 << 15) | (4 << 7) | (1 << 6)
        );
        adc.write(
            Esp32s3SarAdcRegister::FilterCtrl1.offset(),
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(
                Esp32s3SarAdcRegister::FilterCtrl1.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            0xfc00_0000
        );
        assert!(
            adc.write(
                Esp32s3SarAdcRegister::IntRaw.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .is_err()
        );
        assert!(adc.read(0x74, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            adc.write(
                Esp32s3SarAdcRegister::Date.offset(),
                AccessWidth::Word,
                1 << 40,
                SimTime::ZERO,
            )
            .is_err()
        );
    }
}
