use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const CS: u64 = 0x00;
const RESULT: u64 = 0x04;
const FCS: u64 = 0x08;
const FIFO: u64 = 0x0c;
const DIV: u64 = 0x10;
const INTR: u64 = 0x18;
const INTE: u64 = 0x20;
const INTF: u64 = 0x24;
const INTS: u64 = 0x28;
const EN: u32 = 1;
const TS_EN: u32 = 1 << 1;
const START_ONCE: u32 = 1 << 2;
const START_MANY: u32 = 1 << 3;
const READY: u32 = 1 << 8;

#[derive(Clone)]
struct AdcState {
    control: u32,
    result: u16,
    samples: [u16; 5],
    fifo_control: u32,
    divider: u32,
    interrupt_enable: u32,
    interrupt_force: u32,
}

impl Default for AdcState {
    fn default() -> Self {
        Self {
            control: 0,
            result: 0,
            samples: [0; 5],
            fifo_control: 0,
            divider: 0,
            interrupt_enable: 0,
            interrupt_force: 0,
        }
    }
}

/// Host-facing deterministic RP ADC state.
#[derive(Clone)]
pub struct RpAdcHandle(Arc<Mutex<AdcState>>);

impl RpAdcHandle {
    /// Sets the 12-bit sample returned for one ADC input channel.
    pub fn set_sample(&self, channel: usize, value: u16) {
        if let Some(sample) = self
            .0
            .lock()
            .expect("RP ADC lock poisoned")
            .samples
            .get_mut(channel)
        {
            *sample = value & 0x0fff;
        }
    }

    /// Returns the most recently converted sample.
    pub fn result(&self) -> u16 {
        self.0.lock().expect("RP ADC lock poisoned").result
    }
}

/// Functional RP2040/RP2350 ADC and temperature-sensor register subset.
pub struct RpAdc {
    name: String,
    state: Arc<Mutex<AdcState>>,
}

impl RpAdc {
    /// Creates a disabled ADC with deterministic zero-valued inputs.
    pub fn new(name: impl Into<String>) -> (Self, RpAdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RpAdcHandle(state),
        )
    }

    fn access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP ADC requires aligned word access"));
        }
        Ok(())
    }

    fn convert(state: &mut AdcState) {
        let channel = ((state.control >> 12) & 7) as usize;
        state.result = state.samples.get(channel).copied().unwrap_or(0);
        state.control |= READY;
    }
}

impl Device for RpAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::access(offset, width)?;
        let state = self.state.lock().expect("RP ADC lock poisoned");
        let value = match offset {
            CS => state.control,
            RESULT => u32::from(state.result),
            FCS => state.fifo_control | 1 << 24,
            FIFO => u32::from(state.result),
            DIV => state.divider,
            INTR => 0,
            INTE => state.interrupt_enable,
            INTF => state.interrupt_force,
            INTS => state.interrupt_force | state.interrupt_enable,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP ADC read at {offset:#x}"
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
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("ADC value fits");
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        match offset {
            CS => {
                state.control = value & (EN | TS_EN | START_ONCE | START_MANY | (7 << 12));
                if state.control & (START_ONCE | START_MANY) != 0 {
                    Self::convert(&mut state);
                    if state.control & START_MANY == 0 {
                        state.control &= !START_ONCE;
                    }
                }
            }
            FCS => state.fifo_control = value & 0x0fff_ffff,
            DIV => state.divider = value & 0x00ff_ffff,
            INTE => state.interrupt_enable = value,
            INTF => state.interrupt_force = value,
            RESULT | FIFO | INTR | INTS => {
                return Err(DeviceError::new("RP ADC register is read-only"));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP ADC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RP ADC lock poisoned") = AdcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_channel_conversion_sets_ready_and_result() {
        let (mut adc, handle) = RpAdc::new("adc");
        handle.set_sample(3, 0xabc);
        adc.write(
            CS,
            AccessWidth::Word,
            u64::from(EN | START_ONCE | (3 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(CS, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(READY),
            u64::from(READY)
        );
        assert_eq!(
            adc.read(RESULT, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xabc
        );
        assert_eq!(handle.result(), 0xabc);
    }

    #[test]
    fn disabled_adc_still_reports_empty_fifo() {
        let (mut adc, _) = RpAdc::new("adc");
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 24
        );
        assert_eq!(
            adc.read(RESULT, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
    }
}
