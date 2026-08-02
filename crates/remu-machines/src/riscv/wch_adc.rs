use super::{MachineError, RiscVMachine};

impl RiscVMachine {
    /// Routes a completed WCH ADC conversion to its native IRQ 29 line.
    pub(super) fn poll_wch_adc(&mut self) -> Result<(), MachineError> {
        if let Some(wch) = &self.wch {
            wch.pfic
                .set_pending(29, wch.adc.interrupt_pending(self.now));
            let pending = wch.pfic.next_pending() == Some(29);
            self.cpu.set_qingke_external_interrupt(29, pending)?;
        }
        Ok(())
    }
}
