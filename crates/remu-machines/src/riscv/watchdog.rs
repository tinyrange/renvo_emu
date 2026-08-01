use super::{MachineError, RiscVMachine};
use remu_bus::{AddressSpace, MapError};
use remu_core::{Cpu, ResetKind, RunStats};
use remu_devices::{EspLpWatchdog, EspLpWatchdogHandle};

/// Maps the ESP32-C6 LP watchdog at its native address and returns its
/// scheduler-facing handle.
pub(super) fn map_esp32c6_lp_watchdog(
    bus: &mut AddressSpace,
) -> Result<EspLpWatchdogHandle, MapError> {
    let (device, handle) = EspLpWatchdog::new("esp32c6.lp-watchdog");
    bus.map_device("esp32c6.lp-watchdog", 0x600b_1c00, 0x400, Box::new(device))?;
    Ok(handle)
}

impl RiscVMachine {
    /// Dispatches one functional LP-WDT CPU/system reset.
    pub(super) fn poll_esp32c6_watchdog(
        &mut self,
        stats: &mut RunStats,
    ) -> Result<bool, MachineError> {
        if !self
            .esp_lp_watchdog
            .as_ref()
            .is_some_and(|watchdog| watchdog.take_reset(self.now))
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
