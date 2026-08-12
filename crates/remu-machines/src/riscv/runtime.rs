use super::*;

impl RiscVMachine {
    /// Drives or releases one compiler-facade GPIO input.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), MachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        for gpio in &self.chip_gpio {
            if usize::from(pin) < gpio.pin_count() {
                gpio.set_input(pin, value, self.now)?;
            }
        }
        if let Some(peripherals) = &self.esp32c6_peripherals {
            peripherals.observe_pin(pin, value, self.now)?;
        }
        Ok(())
    }

    /// Applies a power-on reset to the CPU and devices.
    pub fn reset(&mut self) -> Result<(), MachineError> {
        self.bus.reset_devices(ResetKind::PowerOn);
        self.cpu.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.cpu1.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.cpu1_active = false;
        self.now = SimTime::ZERO;
        self.esp_cpu_frequency_mhz = 40;
        self.esp_enabled_watchdogs.clear();
        self.esp_interrupt_routes.clear();
        self.esp_enabled_interrupts.clear();
        self.esp_interrupt_priorities.clear();
        self.esp_interrupt_threshold = 0;
        self.esp_md5_contexts.clear();
        self.esp_sha256_contexts.clear();
        self.esp_heaps.clear();
        self.esp_systimer_offset = 0;
        self.esp_systimer_alarms = [u64::MAX; 3];
        self.esp_systimer_periods = [0; 3];
        self.esp_systimer_next = [u64::MAX; 3];
        self.esp_systimer_interrupt_enabled = [false; 3];
        self.esp_systimer_raw = 0;
        self.esp_flash_guard = 0;
        self.esp32c6_materialized_mmu.fill(u32::MAX);
        self.esp32c6_flash_dirty = false;
        self.esp_reset_reason = 1;
        if let Some(peripherals) = &self.esp32c6_peripherals {
            peripherals.lp_clkrst.set_reset_cause(1);
        }
        Ok(())
    }

    pub(super) fn poll_esp32c6_runtime(
        &mut self,
        stats: &mut RunStats,
    ) -> Result<bool, MachineError> {
        self.poll_esp32c6_flash_commands()?;
        // The cache-MMU table is memory-mapped hardware. Real mask-ROM code
        // programs it directly, so consume those writes independently of the
        // functional-ROM compatibility path and expose the selected flash
        // page before the guest's next instruction.
        self.refresh_esp32c6_mmu_mappings()?;
        if self.poll_esp32c6_watchdog(stats)? {
            return Ok(true);
        }
        while let Some((address, size)) = self
            .esp_c6_extmem
            .as_ref()
            .and_then(EspC6ExtmemHandle::take_sync)
        {
            self.refresh_esp32c6_cache(address, size)?;
            stats.events = stats.events.saturating_add(1);
        }
        if self.target != TargetId::Esp32c6 {
            return Ok(false);
        }
        let Some(peripherals) = &self.esp32c6_peripherals else {
            return Ok(false);
        };
        if peripherals.lp_aon.take_system_reset() {
            self.esp_reset_reason = 0x03;
            self.bus.reset_devices(ResetKind::Software);
            peripherals.lp_clkrst.set_reset_cause(0x03);
            self.cpu.reset(ResetKind::Software, &mut self.bus)?;
            self.cpu1.reset(ResetKind::Software, &mut self.bus)?;
            self.cpu1_active = false;
            stats.events = stats.events.saturating_add(1);
            return Ok(true);
        }
        if peripherals.lp_aon.take_cpu_reset() {
            self.esp_reset_reason = 0x0c;
            self.cpu.reset(ResetKind::Software, &mut self.bus)?;
            stats.events = stats.events.saturating_add(1);
            return Ok(true);
        }
        let timer_wakes_lp = peripherals.pmu.lp_wakeup_mask() & (1 << 4) != 0
            && peripherals.lp_timer.lp_wakeup_pending(self.now);
        let wake_lp = peripherals.pmu.take_hp_trigger_lp() || timer_wakes_lp;
        if wake_lp
            && !peripherals.lp_aon.lp_core_disabled()
            && !peripherals.lp_aon.hp_owns_fast_memory()
        {
            self.cpu1.reset(ResetKind::Software, &mut self.bus)?;
            self.cpu1.set_pc(0x5000_0080)?;
            self.cpu1_active = true;
            stats.events = stats.events.saturating_add(1);
        }
        if self.cpu1_active && peripherals.pmu.take_lp_sleep() {
            if peripherals.pmu.lp_reset_on_sleep() {
                self.cpu1.reset(ResetKind::Software, &mut self.bus)?;
            }
            self.cpu1_active = false;
            stats.events = stats.events.saturating_add(1);
        }
        if peripherals.pmu.take_lp_trigger_hp() || peripherals.lp_timer.hp_wakeup_pending(self.now)
        {
            self.cpu.wake_from_wait();
            peripherals.pmu.record_hp_wakeup(1 << 4);
            stats.events = stats.events.saturating_add(1);
        }
        let _ = peripherals.pmu.take_hp_sleep();
        Ok(false)
    }
}
