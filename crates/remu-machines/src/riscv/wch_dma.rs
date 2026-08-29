use super::{MachineError, RiscVMachine};

impl RiscVMachine {
    pub(crate) fn poll_wch_dma(&mut self, events: &mut u64) -> Result<(), MachineError> {
        let Some(wch) = &self.wch else {
            return Ok(());
        };
        *events = events.saturating_add(
            u64::try_from(wch.dma.service(&mut self.bus, self.now)?).expect("DMA count fits u64"),
        );
        for channel in 0..7 {
            let interrupt = 22 + u16::try_from(channel).expect("WCH DMA channel fits");
            let pending = wch.dma.channel_pending(channel);
            wch.pfic.set_pending(interrupt, pending);
            self.cpu.set_qingke_external_interrupt(
                interrupt,
                wch.pfic.next_pending() == Some(interrupt),
            )?;
        }
        Ok(())
    }
}
