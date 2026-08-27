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

    pub(super) fn refresh_pio_dma_requests(&self) -> Result<(), MachineError> {
        let (Some(dma), Some(gpio)) = (&self.dma, self.chip_gpio.first()) else {
            return Ok(());
        };
        for pin in 0..gpio.pin_count() {
            let pin = pin as u8;
            let pull = self
                .pads
                .as_ref()
                .and_then(|pads| pads.pull(pin))
                .unwrap_or(Logic::Z);
            gpio.drive_weak_source(pin, 1, pull, self.now)?;
        }
        for (block, pio) in self.pio.iter().enumerate() {
            let (_, _, gpio_base) = pio.pad_state();
            let mut inputs = 0_u32;
            for logical_pin in 0..32 {
                let physical_pin = usize::from(gpio_base) + logical_pin;
                if physical_pin >= gpio.pin_count() {
                    continue;
                }
                let pin = physical_pin as u8;
                let control = self
                    .io_bank
                    .as_ref()
                    .and_then(|bank| bank.pin_control(pin))
                    .unwrap_or(0x1f);
                let input_enabled = self
                    .pads
                    .as_ref()
                    .is_none_or(|pads| pads.input_enabled(pin));
                let mut high = input_enabled && gpio.resolved(pin)? == Logic::One;
                high = match control >> 16 & 3 {
                    0 => high,
                    1 => !high,
                    2 => false,
                    _ => true,
                };
                inputs |= u32::from(high) << logical_pin;
            }
            pio.set_inputs(inputs);
            for machine in 0..4 {
                let base = block * 8 + machine;
                dma.set_dreq(base as u8, pio.tx_dreq(machine));
                dma.set_dreq((base + 4) as u8, pio.rx_dreq(machine));
            }
        }

        for pin in 0..gpio.pin_count() {
            let pin = pin as u8;
            let control = self
                .io_bank
                .as_ref()
                .and_then(|bank| bank.pin_control(pin))
                .unwrap_or(0x1f);
            let selected = match control & 0x1f {
                function @ 6..=8 => usize::try_from(function - 6)
                    .ok()
                    .filter(|block| *block < self.pio.len()),
                _ => None,
            };
            let bit = if pin < 32 {
                1_u32 << pin
            } else {
                1_u32 << (pin - 32)
            };
            let (sio_direction, sio_output) = if pin < 32 {
                (gpio.direction(), gpio.output())
            } else {
                (gpio.direction_high(), gpio.output_high())
            };
            let output_disabled = self
                .pads
                .as_ref()
                .is_some_and(|pads| pads.output_disabled(pin));
            let sio = if output_disabled || selected.is_some() || sio_direction & bit == 0 {
                Logic::Z
            } else if sio_output & bit == 0 {
                Logic::Zero
            } else {
                Logic::One
            };
            gpio.drive_source(pin, 0, sio, self.now)?;

            for (block, pio) in self.pio.iter().enumerate() {
                let (output, direction, gpio_base) = pio.pad_state();
                let logical = usize::from(pin).checked_sub(usize::from(gpio_base));
                let source_selected =
                    selected == Some(block) && logical.is_some_and(|logical| logical < 32);
                let mut output_enabled =
                    source_selected && direction & (1 << logical.unwrap_or(0)) != 0;
                let mut high = logical
                    .filter(|logical| *logical < 32)
                    .is_some_and(|logical| output & (1 << logical) != 0);
                high = match control >> 8 & 3 {
                    0 => high,
                    1 => !high,
                    2 => false,
                    _ => true,
                };
                output_enabled = match control >> 12 & 3 {
                    0 => output_enabled,
                    1 => !output_enabled,
                    2 => false,
                    _ => true,
                };
                output_enabled &= !output_disabled;
                let logic = if !source_selected || !output_enabled {
                    Logic::Z
                } else if high {
                    Logic::One
                } else {
                    Logic::Zero
                };
                gpio.drive_source(pin, 16 + block as u16, logic, self.now)?;
            }
        }
        Ok(())
    }

    /// Selects the deterministic RP2350 security/privilege context for one hart.
    pub fn set_rp2350_security_context(
        &mut self,
        hart: usize,
        secure: bool,
        privileged: bool,
    ) -> Result<(), MachineError> {
        if self.target != TargetId::Rp2350 {
            return Err(
                remu_bus::DeviceError::new("security context is available only on RP2350").into(),
            );
        }
        let Some(context) = self.security_contexts.get_mut(hart) else {
            return Err(remu_bus::DeviceError::new(format!(
                "RP2350 hart index {hart} is outside 0..2"
            ))
            .into());
        };
        *context = (secure, privileged);
        if let Some(accessctrl) = &self.accessctrl {
            accessctrl.set_context(
                if hart == 0 {
                    Rp2350AccessMaster::Core0
                } else {
                    Rp2350AccessMaster::Core1
                },
                secure,
                privileged,
            );
        }
        Ok(())
    }

    pub(super) fn select_rp2350_access_context(&self, hart: usize) {
        if let Some(accessctrl) = &self.accessctrl {
            let (secure, privileged) = self.security_contexts[hart];
            accessctrl.set_context(
                if hart == 0 {
                    Rp2350AccessMaster::Core0
                } else {
                    Rp2350AccessMaster::Core1
                },
                secure,
                privileged,
            );
        }
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
