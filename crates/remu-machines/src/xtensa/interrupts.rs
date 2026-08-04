use super::{Cpu, XtensaMachine, XtensaMachineError};

impl XtensaMachine {
    fn update_matrix_source(
        &mut self,
        source: usize,
        pending: bool,
    ) -> Result<bool, XtensaMachineError> {
        for core in 0..2_u32 {
            self.interrupt_matrix
                .set_source_pending(core as usize, source, pending);
            let interrupt = self.interrupt_matrix.route(core as usize, source);
            if interrupt == u8::MAX || interrupt == 6 {
                continue;
            }
            if core == 0 {
                self.cpu.set_interrupt(u16::from(interrupt), pending)?;
            } else if self.appcpu_boot_address.is_some() {
                self.cpu1.set_interrupt(u16::from(interrupt), pending)?;
            }
        }
        Ok(pending)
    }

    pub(super) fn update_uhci_interrupt_lines(&mut self) -> Result<bool, XtensaMachineError> {
        self.update_matrix_source(14, self.uhci.interrupt_pending())
    }

    pub(super) fn update_syscon_interrupt_lines(&mut self) -> Result<bool, XtensaMachineError> {
        // SPI_MEM_REJECT_INTR is source 60 in both native matrices.
        self.update_matrix_source(60, self.syscon.interrupt_pending())
    }
}
