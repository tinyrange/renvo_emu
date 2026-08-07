use super::{MachineError, RiscVMachine};
use remu_core::{Cpu, ResetKind, RunStats};

impl RiscVMachine {
    /// Dispatches one functional LP-WDT CPU/system reset.
    pub(super) fn poll_esp32c6_watchdog(
        &mut self,
        stats: &mut RunStats,
    ) -> Result<bool, MachineError> {
        if !self
            .esp32c6_peripherals
            .as_ref()
            .is_some_and(|peripherals| peripherals.lp_watchdog.take_reset(self.now))
        {
            return Ok(false);
        }
        self.bus.reset_devices(ResetKind::Watchdog);
        self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
        self.cpu1.reset(ResetKind::Watchdog, &mut self.bus)?;
        self.cpu1_active = false;
        stats.events = stats.events.saturating_add(1);
        Ok(true)
    }
}
