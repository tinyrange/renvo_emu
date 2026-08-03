use super::*;

impl RiscVMachine {
    /// Supplies a deterministic analog sample to a WCH ADC channel.
    pub fn set_wch_adc_sample(&self, channel: u8, value: u16) -> Result<(), MachineError> {
        let Some(adc) = &self.wch_adc else {
            return Err(MachineError::UnsupportedTarget(self.target));
        };
        adc.set_channel_sample(channel, value);
        Ok(())
    }

    /// Routes a completed WCH ADC conversion to its native IRQ 29 line.
    pub(super) fn poll_wch_adc(&mut self) -> Result<(), MachineError> {
        if let (Some(adc), Some(pfic)) = (&self.wch_adc, &self.wch_pfic) {
            pfic.set_pending(29, adc.interrupt_pending(self.now));
            let pending = pfic.next_pending() == Some(29);
            self.cpu.set_qingke_external_interrupt(29, pending)?;
        }
        Ok(())
    }
}
