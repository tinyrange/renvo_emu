use super::{MachineError, RiscVMachine};
use remu_core::{Cpu, ResetKind, RunStats};
use remu_devices::EspWatchdogAction;

impl RiscVMachine {
    /// Dispatches one functional LP-WDT CPU/system reset.
    pub(super) fn poll_esp32c6_watchdog(
        &mut self,
        stats: &mut RunStats,
    ) -> Result<bool, MachineError> {
        let mut selected = self.esp32c6_peripherals.as_ref().and_then(|peripherals| {
            peripherals
                .lp_watchdog
                .take_action(self.now)
                .map(|action| (action, 0x0d, 0x09))
        });
        for (group, watchdog) in self.esp_timer_groups.iter().enumerate() {
            if selected.is_none()
                && let Some(action) = watchdog.take_watchdog_action(self.now)
            {
                let cpu_reason = if group == 0 { 0x0b } else { 0x11 };
                let system_reason = if group == 0 { 0x07 } else { 0x08 };
                selected = Some((action, cpu_reason, system_reason));
            }
        }
        let Some((action, cpu_reason, system_reason)) = selected else {
            return Ok(false);
        };
        if action == EspWatchdogAction::Interrupt {
            stats.events = stats.events.saturating_add(1);
            return Ok(false);
        }
        if action == EspWatchdogAction::ResetCpu {
            self.esp_reset_reason = cpu_reason;
            if let Some(peripherals) = &self.esp32c6_peripherals {
                peripherals.lp_clkrst.set_reset_cause(cpu_reason);
            }
            self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
            stats.events = stats.events.saturating_add(1);
            return Ok(true);
        }
        self.esp_reset_reason = if action == EspWatchdogAction::ResetRtc {
            0x10
        } else {
            system_reason
        };
        self.bus
            .reset_devices(if action == EspWatchdogAction::ResetRtc {
                ResetKind::PowerOn
            } else {
                ResetKind::Watchdog
            });
        if let Some(peripherals) = &self.esp32c6_peripherals {
            peripherals.lp_clkrst.set_reset_cause(self.esp_reset_reason);
        }
        self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
        if action == EspWatchdogAction::ResetRtc {
            self.cpu1.reset(ResetKind::Watchdog, &mut self.bus)?;
            self.cpu1_active = false;
        }
        if action != EspWatchdogAction::ResetRtc
            && let Some(application) = self.esp_application.clone()
        {
            // Functional mask-ROM boot: a main-watchdog system reset retains
            // flash and the LP always-on stores, then performs the same
            // verified application handoff as the initial boot.
            self.load_esp_application(&application)?;
        } else if action != EspWatchdogAction::ResetRtc
            && let Some(firmware) = self.esp_direct_firmware.clone()
        {
            // Direct ELF mode models the same second-stage boot by restoring
            // its initialized segments and entry point.
            self.load_firmware(&firmware)?;
        }
        stats.events = stats.events.saturating_add(1);
        Ok(true)
    }
}
