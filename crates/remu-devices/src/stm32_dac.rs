use super::*;

const CR: u64 = 0x00;
const SWTRIGR: u64 = 0x04;
const DHR12R1: u64 = 0x08;
const DHR12L1: u64 = 0x0c;
const DHR8R1: u64 = 0x10;
const DHR12R2: u64 = 0x14;
const DHR12L2: u64 = 0x18;
const DHR8R2: u64 = 0x1c;
const DHR12RD: u64 = 0x20;
const DHR12LD: u64 = 0x24;
const DHR8RD: u64 = 0x28;
const DOR1: u64 = 0x2c;
const DOR2: u64 = 0x30;
const SR: u64 = 0x34;
const CCR: u64 = 0x38;
const MCR: u64 = 0x3c;
const SHSR1: u64 = 0x40;
const SHSR2: u64 = 0x44;
const SHHR: u64 = 0x48;
const SHRR: u64 = 0x4c;

const EN1: u32 = 1 << 0;
const TEN1: u32 = 1 << 2;
const EN2: u32 = 1 << 16;
const TEN2: u32 = 1 << 18;

/// Functional STM32L432 DAC1 slice.
///
/// The model keeps the two 12-bit data holding registers, supports the
/// right/left/8-bit write formats, and applies software-triggered transfers
/// when the corresponding trigger is enabled. It emits the current digital
/// output code and enable state as traceable signals; analog voltage,
/// calibration, sample-and-hold settling, and DMA are intentionally outside
/// this functional boundary.
pub struct Stm32Dac {
    name: String,
    registers: [u32; 0x100 / 4],
    holding: [u16; 2],
    output: [u16; 2],
    hub: SignalHub,
    output_signals: [SignalId; 2],
    enable_signals: [SignalId; 2],
}

impl Stm32Dac {
    /// Creates a reset DAC1 with two trace-visible output channels.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, remu_signals::SignalError> {
        let name = name.into();
        let output_signals = [
            hub.declare(
                format!("{name}.channel1.value"),
                SignalValue::from_u64(0, 12)?,
                Some("DAC channel 1 digital output code".to_owned()),
            )?,
            hub.declare(
                format!("{name}.channel2.value"),
                SignalValue::from_u64(0, 12)?,
                Some("DAC channel 2 digital output code".to_owned()),
            )?,
        ];
        let enable_signals = [
            hub.declare(
                format!("{name}.channel1.enabled"),
                SignalValue::from_u64(0, 1)?,
                Some("DAC channel 1 enabled state".to_owned()),
            )?,
            hub.declare(
                format!("{name}.channel2.enabled"),
                SignalValue::from_u64(0, 1)?,
                Some("DAC channel 2 enabled state".to_owned()),
            )?,
        ];
        Ok(Self {
            name,
            registers: [0; 0x100 / 4],
            holding: [0; 2],
            output: [0; 2],
            hub,
            output_signals,
            enable_signals,
        })
    }

    fn index(offset: u64) -> Result<usize, DeviceError> {
        if offset & 3 != 0 {
            return Err(DeviceError::new("STM32 DAC requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("DAC offset fits usize");
        (index < 0x100 / 4)
            .then_some(index)
            .ok_or_else(|| DeviceError::new(format!("STM32 DAC access at {offset:#x}")))
    }

    fn channel_enabled(&self, channel: usize) -> bool {
        let mask = if channel == 0 { EN1 } else { EN2 };
        self.registers[(CR / 4) as usize] & mask != 0
    }

    fn triggered(&self, channel: usize) -> bool {
        let mask = if channel == 0 { TEN1 } else { TEN2 };
        self.registers[(CR / 4) as usize] & mask != 0
    }

    fn publish(&self, channel: usize, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.output_signals[channel],
                SignalValue::from_u64(u64::from(self.output[channel]), 12)
                    .expect("DAC output signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))?;
        self.hub
            .set(
                self.enable_signals[channel],
                SignalValue::from_u64(u64::from(self.channel_enabled(channel)), 1)
                    .expect("DAC enable signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn transfer(&mut self, channel: usize, at: SimTime) -> Result<(), DeviceError> {
        self.output[channel] = self.holding[channel];
        self.registers[((DOR1 + 4 * channel as u64) / 4) as usize] =
            u32::from(self.output[channel]);
        self.publish(channel, at)
    }

    fn write_holding(
        &mut self,
        channel: usize,
        value: u32,
        format: DacDataFormat,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.holding[channel] = format.decode(value);
        if !self.triggered(channel) {
            self.transfer(channel, at)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum DacDataFormat {
    Right12,
    Left12,
    Right8,
}

impl DacDataFormat {
    fn decode(self, value: u32) -> u16 {
        match self {
            Self::Right12 => (value & 0x0fff) as u16,
            Self::Left12 => ((value >> 4) & 0x0fff) as u16,
            Self::Right8 => ((value & 0xff) << 4) as u16,
        }
    }
}

impl Device for Stm32Dac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 DAC requires word accesses"));
        }
        let index = Self::index(offset)?;
        let value = match offset {
            DOR1 => u32::from(self.output[0]),
            DOR2 => u32::from(self.output[1]),
            _ => self.registers[index],
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
            return Err(DeviceError::new("STM32 DAC requires word accesses"));
        }
        let index = Self::index(offset)?;
        let value = value as u32;
        match offset {
            CR => {
                self.registers[index] = value;
                self.publish(0, at)?;
                self.publish(1, at)?;
            }
            SWTRIGR => {
                self.registers[index] = value & 0x3;
                if value & 1 != 0 && self.triggered(0) {
                    self.transfer(0, at)?;
                }
                if value & 2 != 0 && self.triggered(1) {
                    self.transfer(1, at)?;
                }
            }
            DHR12R1 => self.write_holding(0, value, DacDataFormat::Right12, at)?,
            DHR12L1 => self.write_holding(0, value, DacDataFormat::Left12, at)?,
            DHR8R1 => self.write_holding(0, value, DacDataFormat::Right8, at)?,
            DHR12R2 => self.write_holding(1, value, DacDataFormat::Right12, at)?,
            DHR12L2 => self.write_holding(1, value, DacDataFormat::Left12, at)?,
            DHR8R2 => self.write_holding(1, value, DacDataFormat::Right8, at)?,
            DHR12RD => {
                self.write_holding(0, value, DacDataFormat::Right12, at)?;
                self.write_holding(1, value >> 16, DacDataFormat::Right12, at)?;
            }
            DHR12LD => {
                self.write_holding(0, value, DacDataFormat::Left12, at)?;
                self.write_holding(1, value >> 16, DacDataFormat::Left12, at)?;
            }
            DHR8RD => {
                self.write_holding(0, value, DacDataFormat::Right8, at)?;
                self.write_holding(1, value >> 8, DacDataFormat::Right8, at)?;
            }
            DOR1 | DOR2 | SR => {}
            CCR | MCR | SHSR1 | SHSR2 | SHHR | SHRR => self.registers[index] = value,
            _ => self.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers = [0; 0x100 / 4];
        self.holding = [0; 2];
        self.output = [0; 2];
        let _ = self.publish(0, SimTime::ZERO);
        let _ = self.publish(1, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_support_data_formats_and_software_trigger() {
        let hub = SignalHub::new();
        let mut dac = Stm32Dac::new("board.stm32l432kc.dac1", hub.clone()).unwrap();
        dac.write(
            CR,
            AccessWidth::Word,
            u64::from(EN1 | TEN1 | EN2),
            SimTime::ZERO,
        )
        .unwrap();
        dac.write(DHR12R1, AccessWidth::Word, 0xabc, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(dac.read(DOR1, AccessWidth::Word, SimTime::ZERO), Ok(0));
        dac.write(SWTRIGR, AccessWidth::Word, 1, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(dac.read(DOR1, AccessWidth::Word, SimTime::ZERO), Ok(0xabc));
        dac.write(DHR8R2, AccessWidth::Word, 0x5a, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(dac.read(DOR2, AccessWidth::Word, SimTime::ZERO), Ok(0x5a0));
        let signal = hub
            .with_registry(|registry| registry.find("board.stm32l432kc.dac1.channel1.value"))
            .unwrap();
        assert_eq!(
            hub.with_registry(|registry| registry.value(signal).unwrap().to_vcd_binary()),
            "101010111100"
        );
    }

    #[test]
    fn disabled_channel_still_latches_data_but_publishes_enable_state() {
        let hub = SignalHub::new();
        let mut dac = Stm32Dac::new("dac1", hub.clone()).unwrap();
        dac.write(DHR12L1, AccessWidth::Word, 0xabc0, SimTime::ZERO)
            .unwrap();
        assert_eq!(dac.read(DOR1, AccessWidth::Word, SimTime::ZERO), Ok(0xabc));
        let signal = hub
            .with_registry(|registry| registry.find("dac1.channel1.enabled"))
            .unwrap();
        assert_eq!(
            hub.with_registry(|registry| registry.value(signal).unwrap().to_vcd_binary()),
            "0"
        );
    }
}
