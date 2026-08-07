use super::*;

const CTRL: usize = 0x00;
const SAR1_STATUS: usize = 0x10;
const SAR2_STATUS: usize = 0x14;
const ONETIME_SAMPLE: usize = 0x20;
const SAR1_DATA: usize = 0x2c;
const SAR2_DATA: usize = 0x30;
const INT_ENA: usize = 0x40;
const INT_RAW: usize = 0x44;
const INT_ST: usize = 0x48;
const INT_CLR: usize = 0x4c;
const TSENS_CTRL: usize = 0x58;
const TSENS_CTRL2: usize = 0x5c;
const CALI: usize = 0x60;
const TSENS_SAMPLE: usize = 0x68;
const DATE: usize = 0x3fc;

/// Functional ESP32-C6 SAR ADC and temperature-sensor register block.
pub struct EspSarAdc {
    name: String,
    state: Arc<Mutex<EspSarAdcState>>,
}

struct EspSarAdcState {
    registers: Vec<u32>,
    samples: [u16; 2],
    temperature: u8,
    hub: SignalHub,
    signals: [SignalId; 3],
}

impl EspSarAdc {
    /// Creates the native APB SAR ADC block and its VCD-backed observations.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let signals = [
            hub.declare(
                "board.esp32c6.saradc.adc1",
                SignalValue::from_u64(0, 13)?,
                Some("ESP32-C6 SAR ADC1 sample".to_owned()),
            )?,
            hub.declare(
                "board.esp32c6.saradc.adc2",
                SignalValue::from_u64(0, 13)?,
                Some("ESP32-C6 SAR ADC2 sample".to_owned()),
            )?,
            hub.declare(
                "board.esp32c6.saradc.temperature",
                SignalValue::from_u64(128, 8)?,
                Some("ESP32-C6 temperature sensor code".to_owned()),
            )?,
        ];
        let state = EspSarAdcState {
            registers: vec![0; 0x1000 / 4],
            samples: [0; 2],
            temperature: 128,
            hub,
            signals,
        };
        let mut device = Self {
            name: name.into(),
            state: Arc::new(Mutex::new(state)),
        };
        device.reset(ResetKind::PowerOn);
        Ok(device)
    }
}

impl EspSarAdcState {
    fn publish(
        &self,
        index: usize,
        value: u64,
        width: u16,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.signals[index],
                SignalValue::from_u64(value, width).expect("fixed SAR ADC signal width"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn sample(
        &mut self,
        adc: usize,
        channel: u32,
        attenuation: u32,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let value = (2_048_u32
            .saturating_add(channel.saturating_mul(64))
            .saturating_add(attenuation.saturating_mul(512)))
        .min(8_191) as u16;
        self.samples[adc] = value;
        let data = if adc == 0 { SAR1_DATA } else { SAR2_DATA };
        let status = if adc == 0 { SAR1_STATUS } else { SAR2_STATUS };
        self.registers[data / 4] = u32::from(value);
        self.registers[status / 4] = u32::from(value);
        self.registers[INT_RAW / 4] |= 1 << (31 - adc);
        self.registers[INT_ST / 4] = self.registers[INT_RAW / 4] & self.registers[INT_ENA / 4];
        self.publish(adc, u64::from(value), 13, at)
    }

    fn trigger(&mut self, value: u32, at: SimTime) -> Result<(), DeviceError> {
        let channel = (value >> 25) & 0xf;
        let attenuation = (value >> 23) & 3;
        if value & (1 << 31) != 0 {
            self.sample(0, channel, attenuation, at)?;
        }
        if value & (1 << 30) != 0 {
            self.sample(1, channel, attenuation, at)?;
        }
        Ok(())
    }
}

impl Device for EspSarAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP SAR ADC requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("SAR ADC offset fits");
        let state = self.state.lock().expect("ESP SAR ADC lock poisoned");
        let value = *state
            .registers
            .get(offset / 4)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP SAR ADC requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("SAR ADC offset fits");
        let value = value as u32;
        let mut state = self.state.lock().expect("ESP SAR ADC lock poisoned");
        if offset >= state.registers.len() * 4 {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        match offset {
            CTRL => {
                state.registers[CTRL / 4] = value;
                if value & (1 << 1) != 0 {
                    let sample_config = state.registers[ONETIME_SAMPLE / 4] | (1 << 31);
                    state.trigger(sample_config, at)?;
                }
            }
            ONETIME_SAMPLE => {
                state.registers[offset / 4] = value & 0xe7ff_ffff;
                if value & (1 << 29) != 0 {
                    state.trigger(value | (1 << 31), at)?;
                }
                if value & (1 << 30) != 0 {
                    state.trigger(value | (1 << 30), at)?;
                }
            }
            INT_RAW | INT_ST => {}
            INT_ENA => {
                state.registers[INT_ENA / 4] = value & 0xc000_0000;
                state.registers[INT_ST / 4] =
                    state.registers[INT_RAW / 4] & state.registers[INT_ENA / 4];
            }
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value & 0xc000_0000);
                state.registers[INT_ST / 4] =
                    state.registers[INT_RAW / 4] & state.registers[INT_ENA / 4];
            }
            TSENS_CTRL => {
                state.registers[offset / 4] = value & 0x00c0_3fff | u32::from(state.temperature);
            }
            DATE => {}
            _ => state.registers[offset / 4] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP SAR ADC lock poisoned");
        state.registers.fill(0);
        state.samples = [0; 2];
        state.temperature = 128;
        state.registers[SAR1_STATUS / 4] = 1 << 29;
        state.registers[SAR2_STATUS / 4] = 1 << 29;
        state.registers[0x18 / 4] = 0x00ff_ffff;
        state.registers[0x1c / 4] = 0x00ff_ffff;
        state.registers[ONETIME_SAMPLE / 4] = 13 << 25;
        state.registers[TSENS_CTRL / 4] = (6 << 14) | 128;
        state.registers[TSENS_CTRL2 / 4] = 1 << 14;
        state.registers[CALI / 4] = 32_768;
        state.registers[TSENS_SAMPLE / 4] = 20;
        state.registers[DATE / 4] = 35_676_736;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_adc_sample_is_deterministic_and_interruptible() {
        let hub = SignalHub::new();
        let mut adc = EspSarAdc::new("saradc", hub).unwrap();
        adc.write(INT_ENA as u64, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        adc.write(
            ONETIME_SAMPLE as u64,
            AccessWidth::Word,
            (3 << 25) | (1 << 23) | (1 << 29) | (1 << 31),
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert_eq!(
            adc.read(SAR1_DATA as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            2_752
        );
        assert_eq!(
            adc.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 31
        );
        adc.write(INT_CLR as u64, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn temperature_and_reset_defaults_match_c6_header_slice() {
        let hub = SignalHub::new();
        let mut adc = EspSarAdc::new("saradc", hub).unwrap();
        assert_eq!(
            adc.read(TSENS_CTRL as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & 0xff,
            128
        );
        assert_eq!(
            adc.read(DATE as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            35_676_736
        );
    }
}
