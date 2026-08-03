//! STM32L4 ADC1 functional conversion subset.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const ISR: u64 = 0x00;
const IER: u64 = 0x04;
const CR: u64 = 0x08;
const CFGR: u64 = 0x0c;
const SMPR1: u64 = 0x14;
const SMPR2: u64 = 0x18;
const SQR1: u64 = 0x30;
const SQR4: u64 = 0x3c;
const DR: u64 = 0x40;

const ADRDY: u32 = 1 << 0;
const EOC: u32 = 1 << 2;
const EOS: u32 = 1 << 3;
const OVR: u32 = 1 << 4;
const EOCAL: u32 = 1 << 11;
const LDORDY: u32 = 1 << 12;
const ADEN: u32 = 1 << 0;
const ADDIS: u32 = 1 << 1;
const ADSTART: u32 = 1 << 2;
const ADCAL: u32 = 1 << 31;

#[derive(Default)]
struct AdcState {
    isr: u32,
    ier: u32,
    cr: u32,
    cfgr: u32,
    smpr1: u32,
    smpr2: u32,
    sequence: [u32; 4],
    inputs: [u16; 18],
    data: u16,
}

impl AdcState {
    fn selected_channel(&self) -> usize {
        usize::try_from((self.sequence[0] >> 6) & 0x1f)
            .expect("ADC channel index fits usize")
            .min(self.inputs.len() - 1)
    }

    fn convert(&mut self) {
        if self.cr & ADEN == 0 {
            return;
        }
        self.data = self.inputs[self.selected_channel()] & 0x0fff;
        self.isr |= EOC | EOS;
        self.cr &= !ADSTART;
    }
}

/// Host-facing STM32 ADC1 input and conversion state.
#[derive(Clone)]
pub struct Stm32AdcHandle(Arc<Mutex<AdcState>>);

impl Stm32AdcHandle {
    /// Sets one deterministic analog sample in the 12-bit ADC input range.
    pub fn set_input(&self, channel: u8, value: u16) {
        if let Some(input) = self
            .0
            .lock()
            .expect("STM32 ADC lock poisoned")
            .inputs
            .get_mut(usize::from(channel))
        {
            *input = value & 0x0fff;
        }
    }

    /// Returns the most recently converted sample.
    pub fn value(&self) -> u16 {
        self.0.lock().expect("STM32 ADC lock poisoned").data
    }
}

/// Functional STM32L432 ADC1 register block.
pub struct Stm32Adc {
    name: String,
    state: Arc<Mutex<AdcState>>,
}

impl Stm32Adc {
    /// Creates a reset-state ADC1 and host handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32AdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32AdcHandle(state),
        )
    }
}

impl Device for Stm32Adc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 ADC requires aligned word access"));
        }
        let state = self.state.lock().expect("STM32 ADC lock poisoned");
        let value = match offset {
            ISR => state.isr,
            IER => state.ier,
            CR => state.cr & !ADCAL,
            CFGR => state.cfgr,
            SMPR1 => state.smpr1,
            SMPR2 => state.smpr2,
            SQR1..=SQR4 => state.sequence[usize::try_from((offset - SQR1) / 4).unwrap()],
            DR => u32::from(state.data),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 ADC read at {offset:#x}"
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 ADC requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 ADC lock poisoned");
        let value = value as u32;
        match offset {
            ISR => {
                state.isr &= !(value & (ADRDY | EOC | EOS | OVR | EOCAL));
            }
            IER => state.ier = value & (ADRDY | EOC | EOS | OVR | EOCAL),
            CR => {
                if value & ADCAL != 0 {
                    state.isr |= EOCAL;
                }
                if value & ADDIS != 0 {
                    state.cr &= !ADEN;
                    state.isr &= !ADRDY;
                }
                if value & ADEN != 0 {
                    state.cr |= ADEN;
                    state.isr |= ADRDY | LDORDY;
                }
                // ADSTART is ignored while the ADC is disabled.
                if value & ADSTART != 0 && state.cr & ADEN != 0 {
                    state.cr |= ADSTART;
                    state.convert();
                }
                state.cr = (state.cr & (ADEN | ADSTART)) | (value & !(ADCAL | ADSTART | ADDIS));
            }
            CFGR => state.cfgr = value,
            SMPR1 => state.smpr1 = value,
            SMPR2 => state.smpr2 = value,
            SQR1..=SQR4 => state.sequence[usize::try_from((offset - SQR1) / 4).unwrap()] = value,
            DR => return Err(DeviceError::new("STM32 ADC data register is read-only")),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 ADC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 ADC lock poisoned") = AdcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_uses_selected_channel_and_sets_completion_flags() {
        let (mut adc, handle) = Stm32Adc::new("adc1");
        handle.set_input(3, 0x2345);
        adc.write(SQR1, AccessWidth::Word, 3 << 6, SimTime::ZERO)
            .unwrap();
        adc.write(CR, AccessWidth::Word, ADEN.into(), SimTime::ZERO)
            .unwrap();
        adc.write(CR, AccessWidth::Word, ADSTART.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0x345);
        assert_eq!(
            adc.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & (EOC | EOS),
            EOC | EOS
        );
    }

    #[test]
    fn calibration_and_flag_clear_follow_native_control_flow() {
        let (mut adc, _) = Stm32Adc::new("adc1");
        adc.write(CR, AccessWidth::Word, ADCAL.into(), SimTime::ZERO)
            .unwrap();
        assert_ne!(
            adc.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & EOCAL,
            0
        );
        adc.write(ISR, AccessWidth::Word, EOCAL.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & EOCAL,
            0
        );
    }

    #[test]
    fn start_is_ignored_until_adc_is_enabled() {
        let (mut adc, _) = Stm32Adc::new("adc1");
        adc.write(CR, AccessWidth::Word, ADSTART.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(CR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & ADSTART,
            0
        );
    }
}
