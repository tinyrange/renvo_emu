use super::{MachineError, RiscVMachine};
use remu_devices::WchWatchdogEvent;

impl RiscVMachine {
    /// Advances WCH watchdogs and routes the WWDG early-warning level through
    /// the shared PFIC model. A timeout is consumed by the run loop as a
    /// watchdog reset, matching the other MCU machine implementations.
    pub(super) fn poll_wch_watchdogs(&mut self) -> Result<bool, MachineError> {
        let Some(wch) = &self.wch else {
            return Ok(false);
        };
        let iwdg_reset = self
            .wch
            .as_ref()
            .expect("checked WCH handles")
            .watchdogs
            .first()
            .is_some_and(|watchdog| watchdog.take_reset(self.now));
        let wwdg_event = wch
            .watchdogs
            .get(1)
            .map_or(WchWatchdogEvent::default(), |watchdog| {
                watchdog.poll(self.now)
            });
        const WWDG_INTERRUPT: u16 = 16;
        wch.pfic
            .set_pending(WWDG_INTERRUPT, wwdg_event.early_warning);
        let deliver = wch.pfic.next_pending() == Some(WWDG_INTERRUPT);
        self.cpu
            .set_qingke_external_interrupt(WWDG_INTERRUPT, deliver)?;
        Ok(iwdg_reset || wwdg_event.reset)
    }
}
