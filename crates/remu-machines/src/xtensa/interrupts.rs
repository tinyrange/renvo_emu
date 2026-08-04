use super::{Cpu, XtensaMachine, XtensaMachineError, appcpu_systimer_level};

impl XtensaMachine {
    pub(super) fn set_systimer_interrupt(
        &mut self,
        core: u32,
        interrupt: u32,
        pending: bool,
    ) -> Result<(), XtensaMachineError> {
        if core == 0 {
            self.cpu.set_interrupt(interrupt as u16, pending)?;
        } else if core == 1 && self.appcpu_boot_address.is_some() {
            // Retain a CPU1 tick until WAITI or another shallow logical-window
            // safe point while an external script is running.
            let asserted = appcpu_systimer_level(
                pending,
                self.usb_host.input_started(),
                self.cpu1.functional_interrupt_safe_point(),
            );
            self.cpu1.set_interrupt(interrupt as u16, asserted)?;
        }
        Ok(())
    }

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
            let asserted =
                pending && !(interrupt == 14 && self.world_controller.nmi_masked(core as u8));
            if core == 0 {
                self.cpu.set_interrupt(u16::from(interrupt), asserted)?;
            } else if self.appcpu_boot_address.is_some() {
                self.cpu1.set_interrupt(u16::from(interrupt), asserted)?;
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

    pub(super) fn update_extmem_interrupt_lines(&mut self) -> Result<bool, XtensaMachineError> {
        let mut any_pending = false;
        for source in [56, 61, 62, 63, 64] {
            let pending = self.extmem.interrupt_pending(source);
            any_pending |= self.update_matrix_source(source, pending)?;
        }
        Ok(any_pending)
    }

    pub(super) fn update_pms_interrupt_lines(&mut self) -> Result<bool, XtensaMachineError> {
        let mut any_pending = false;
        for source in 84..=93 {
            let pending = self.pms.interrupt_pending(source);
            any_pending |= self.update_matrix_source(source, pending)?;
        }
        Ok(any_pending)
    }
}
