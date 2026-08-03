use super::*;

impl RiscVMachine {
    /// Sets a deterministic sample for the RP2350 ADC channel.
    pub fn set_adc_sample(&self, channel: usize, value: u16) -> bool {
        self.chip_adc
            .as_ref()
            .is_some_and(|adc| adc.set_sample(channel, value))
    }

    /// Returns the most recent RP2350 ADC conversion result.
    pub fn adc_result(&self) -> Option<u16> {
        self.chip_adc.as_ref().map(RpAdcHandle::result)
    }
}
