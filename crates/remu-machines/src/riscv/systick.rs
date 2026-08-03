use super::*;

impl RiscVMachine {
    /// Advances the WCH SysTick-compatible block and exposes IRQ 12 to QingKe.
    pub(super) fn poll_wch_systick(&mut self) -> Result<(), MachineError> {
        if let Some(pfic) = &self.wch_pfic {
            let pending = pfic.take_systick_pending(self.now);
            self.cpu.set_qingke_external_interrupt(12, pending)?;
        }
        Ok(())
    }
}
