use super::{MachineError, RiscVMachine};

impl RiscVMachine {
    /// Advances the WCH SysTick-compatible block and exposes IRQ 12 to QingKe.
    pub(super) fn poll_wch_systick(&mut self) -> Result<(), MachineError> {
        if let Some(wch) = &self.wch {
            let pending = wch.pfic.take_systick_pending(self.now);
            self.cpu.set_qingke_external_interrupt(12, pending)?;
        }
        Ok(())
    }
}
