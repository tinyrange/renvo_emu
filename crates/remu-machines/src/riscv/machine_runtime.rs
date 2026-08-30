impl RiscVMachine {
    /// Selected target.
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.bus.set_access_observer(observer);
    }

    /// Adds a streaming completed-access observer without replacing existing diagnostics.
    pub fn add_access_observer(&mut self, observer: SharedBusAccessObserver) {
        self.bus.add_access_observer(observer);
    }

    /// Returns completed bus operations when recording is enabled.
    pub fn access_log(&self) -> &[remu_bus::BusAccessRecord] {
        self.bus.access_log()
    }

    /// Stops before executing an instruction at `address`.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
    }

    /// Removes one debugger execution breakpoint.
    pub fn remove_breakpoint(&mut self, address: u64) {
        self.breakpoints.remove(&address);
    }

    /// Returns the current CPU0 snapshot for debugger adapters.
    pub fn debug_snapshot(&self) -> CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Reads guest-visible bytes for a debugger.
    pub fn debug_read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (0..length)
            .map(|offset| {
                self.bus
                    .read(
                        address.saturating_add(offset as u64),
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map(|value| value as u8)
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    /// Writes guest-visible bytes for a debugger.
    pub fn debug_write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().enumerate() {
            self.bus
                .write(
                    address.saturating_add(offset as u64),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Stops after a completed CPU data access overlaps `address`.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Returns the machine's shared signal hub for board endpoint attachment.
    pub fn signal_hub(&self) -> SignalHub {
        self.signals.clone()
    }

    /// Stops after the next completed write overlapping `address`.
    pub fn add_write_watchpoint(&mut self, address: u64) {
        self.bus.add_write_watchpoint(address);
    }

    /// Stops after a matching completed write overlaps `address`.
    pub fn add_masked_write_watchpoint(&mut self, address: u64, mask: u64, expected: u64) {
        self.bus
            .add_masked_write_watchpoint(address, mask, expected);
    }

    /// Stops when the named signal satisfies `edge`.
    pub fn add_signal_stop(&mut self, path: &str, edge: SignalEdge) -> Result<(), MachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Removes configured user breakpoints and data watchpoints.
    pub fn clear_debug_stops(&mut self) {
        self.breakpoints.clear();
        self.bus.clear_watchpoints();
        self.signal_stops.clear();
    }

    /// Runs until a terminal condition and optionally streams signal changes.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, MachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped external GPIO stimulus.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, MachineError> {
        if !limits.is_bounded() {
            return Err(MachineError::MissingRunLimit);
        }

        let mut control = RunControl::new(limits, stimuli);
        control.begin_trace(&self.signals, &mut trace)?;

        let mut stats = RunStats {
            instructions: 0,
            time: self.now,
            events: 0,
        };
        self.cpu
            .set_interrupt(TIMER_INTERRUPT, self.timer.pending())?;
        let mut timer_was_pending = false;
        let mut wch_timer_was_pending = false;
        let mut wch_timer1_was_pending = false;
        let mut wch_exti_was_pending = false;
        let mut wch_spi_was_pending = false;
        let mut wch_i2c_was_pending = [false; 2];
        let mut wch_touch_was_pending = false;
        let mut chip_timer_was_pending = 0_u16;
        let mut io_bank_was_pending = false;
        let mut trng_was_pending = false;
        let mut esp_crosscore_was_pending = false;
        let mut esp_usb_was_pending = false;
        let mut esp_timer_was_pending = [[false; 2]; 2];
        let mut pio_runtime_dirty = self.target == TargetId::Rp2350;
        let mut rp2350_io_bank_dirty = self.target == TargetId::Rp2350;
        let mut native_peripherals_active = self.target != TargetId::Esp32c6
            || !stimuli.is_empty()
            || self.stop_on_usb_input_complete
            || self
                .bus
                .take_device_access()
                .is_some_and(|address| address < TEST_GPIO);
        let breakpoints_active = !self.breakpoints.is_empty();
        let watchpoints_active = self.bus.has_watchpoints();
        let reason = loop {
            if let Some(sio) = &self.sio {
                sio.select_core(0);
            }
            let mut stimulus_applied = false;
            control.apply_stimuli(self.now, &mut stats, |stimulus| {
                stimulus_applied = true;
                self.set_pin(stimulus.pin, stimulus.value)
            })?;
            pio_runtime_dirty |= stimulus_applied && self.target == TargetId::Rp2350;
            if let Some(code) = self.exit.code() {
                let _ = code;
                break StopReason::Halted;
            }
            if self.stop_on_usb_input_complete
                && (self
                    .usb_host
                    .as_ref()
                    .is_some_and(Rp2040UsbHost::input_complete)
                    || self
                        .esp_usb_serial_jtag
                        .as_ref()
                        .is_some_and(EspUsbSerialJtagHandle::input_complete))
            {
                break StopReason::HostInputComplete;
            }
            if let Some(reason) = control.limit_reason(self.now, &stats) {
                break reason;
            }
            let pio_active = self.pio.iter().any(RpPioHandle::enabled);
            if pio_runtime_dirty || pio_active {
                self.refresh_pio_dma_requests()?;
                pio_runtime_dirty = false;
                rp2350_io_bank_dirty = self.target == TargetId::Rp2350;
            }
            if let Some(dma) = &self.dma {
                let accessctrl = self.accessctrl.clone();
                let dma_events = dma.service_with_context(
                    &mut self.bus,
                    self.now,
                    move |_, secure, privileged| {
                        if let Some(accessctrl) = &accessctrl {
                            accessctrl.set_context(
                                Rp2350AccessMaster::Dma,
                                secure,
                                privileged,
                            );
                        }
                    },
                )?;
                stats.events = stats.events.saturating_add(dma_events as u64);
                pio_runtime_dirty |= dma_events != 0;
            }
            if breakpoints_active && self.breakpoints.contains(&u64::from(self.cpu.pc())) {
                break StopReason::Breakpoint;
            }
            if native_peripherals_active && self.poll_esp32c6_runtime(&mut stats)? {
                continue;
            }
            if native_peripherals_active {
                stats.events = stats.events.saturating_add(u64::from(
                    self.esp_usb_serial_jtag
                        .as_ref()
                        .is_some_and(|usb| usb.poll(self.now)),
                ));
            }
            if self.poll_wch_watchdogs()? {
                self.bus.reset_devices(ResetKind::Watchdog);
                self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
                stats.events = stats.events.saturating_add(1);
                continue;
            }
            self.poll_wch_systick()?;
            self.poll_wch_adc()?;
            self.poll_wch_touch_key(&mut stats, &mut wch_touch_was_pending)?;
            self.poll_wch_dma(&mut stats.events)?;
            if timer_was_pending || self.timer.active() {
                let timer_pending = self.timer.poll(self.now);
                if timer_pending && !timer_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                timer_was_pending = timer_pending;
                self.cpu.set_interrupt(TIMER_INTERRUPT, timer_pending)?;
            }
            if let Some(wch) = &self.wch {
                wch_exti::poll_wch(
                    wch,
                    &self.chip_gpio,
                    &mut self.cpu,
                    &mut wch_timer_was_pending,
                    &mut wch_timer1_was_pending,
                    &mut wch_exti_was_pending,
                    &mut wch_spi_was_pending,
                    &mut wch_i2c_was_pending,
                    &mut stats,
                    self.now,
                )?;
            }
            if self.target == TargetId::Rp2350 {
                if rp2350_io_bank_dirty {
                    rp_io::poll(self, &mut stats, &mut io_bank_was_pending)?;
                    rp2350_io_bank_dirty = false;
                }
                if let Some(dma) = &self.dma {
                    for interrupt in 0..4 {
                        self.cpu.set_hazard3_external_interrupt(
                            10 + u16::try_from(interrupt).expect("RP2350 DMA IRQ index fits u16"),
                            dma.interrupt_pending(interrupt),
                        )?;
                    }
                }
                if let Some(trng) = &self.trng {
                    let pending = trng.interrupt_pending();
                    if pending && !trng_was_pending {
                        stats.events = stats.events.saturating_add(1);
                    }
                    trng_was_pending = pending;
                    self.cpu.set_hazard3_external_interrupt(39, pending)?;
                }
                let chip_timer_pending =
                    self.chip_timers
                        .iter()
                        .enumerate()
                        .fold(0_u16, |pending, (timer, handle)| {
                            pending | (u16::from(handle.pending(self.now)) << (timer * 4))
                        });
                stats.events = stats.events.saturating_add(u64::from(
                    (chip_timer_pending & !chip_timer_was_pending).count_ones(),
                ));
                chip_timer_was_pending = chip_timer_pending;
                for pio in &self.pio {
                    if pio.poll(self.now)? {
                        stats.events = stats.events.saturating_add(1);
                    }
                }
                if pio_runtime_dirty || pio_active {
                    self.refresh_pio_dma_requests()?;
                    pio_runtime_dirty = false;
                    rp2350_io_bank_dirty = true;
                }
                for line in 0..self.chip_timers.len() * 4 {
                    self.cpu.set_hazard3_external_interrupt(
                        u16::try_from(line).expect("RP timer IRQ line fits u16"),
                        chip_timer_pending & (1 << line) != 0,
                    )?;
                }
                set_rp2350_spi_interrupts(&mut self.cpu, &self.spi)?;
                for (index, handle) in self.i2c.iter().enumerate() {
                    let line = 36_u16 + u16::try_from(index).expect("RP2350 I²C index fits u16");
                    self.cpu.set_hazard3_external_interrupt(line, handle.pending())?;
                }
                if let Some(usb) = &self.usb {
                    if let (Some(host), Some(dpram)) = (&mut self.usb_host, &self.usb_dpram) {
                        stats.events = stats.events.saturating_add(host.poll(self.now, usb, dpram));
                    }
                    self.cpu
                        .set_hazard3_external_interrupt(14, usb.interrupt_pending())?;
                }
            }
            if self.target == TargetId::Esp32c6 && native_peripherals_active {
                stats.events = stats.events.saturating_add(self.service_radio()?);
                // ESP-IDF starts the first FreeRTOS task by raising the
                // FROM_CPU_INTR0 software interrupt. The C6 interrupt matrix
                // routes source 22 to a local CPU interrupt configured by the
                // ROM calls retained above.
                let crosscore_pending = self
                    .bus
                    .read(
                        0x600c_5090,
                        remu_core::AccessWidth::Word,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)?
                    != 0;
                if let Some(peripherals) = &self.esp32c6_peripherals {
                    peripherals
                        .interrupt_matrix
                        .set_source(22, crosscore_pending);
                }
                // Interrupt allocation may program the native matrix register
                // directly, so the device is authoritative over the ROM-call
                // observation cache.
                let interrupt = self
                    .esp32c6_peripherals
                    .as_ref()
                    .and_then(|peripherals| peripherals.interrupt_matrix.route(22))
                    .map(u32::from)
                    .or_else(|| self.esp_interrupt_routes.get(&22).copied())
                    .unwrap_or(2);
                let priority = if interrupt < 32 {
                    self.bus
                        .read(
                            u64::from(0x2000_1010_u32 + interrupt * 4),
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32
                } else {
                    0
                };
                let threshold = self
                    .bus
                    .read(
                        0x2000_1090,
                        remu_core::AccessWidth::Word,
                        remu_core::AccessKind::Read,
                        self.now,
                    )
                    .map_err(MachineError::Bus)? as u32;
                // The guest may program `mie` directly instead of using a ROM
                // helper. Assert an eligible controller line here and let the
                // CPU's architectural MIE state decide whether it is taken.
                let deliver = crosscore_pending && priority >= threshold;
                if deliver && !esp_crosscore_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                esp_crosscore_was_pending = deliver;
                if interrupt < 32 {
                    if let Some(plic) = &self.esp_c6_plic {
                        plic.set_line(interrupt as u8, crosscore_pending);
                    }
                    self.cpu.set_interrupt(interrupt as u16, deliver)?;
                }
                for (group, handle) in self.esp_timer_groups.iter().enumerate() {
                    for (timer, pending) in handle.pending(self.now).into_iter().enumerate() {
                        let source = match (group, timer) {
                            (0, 0) => 51,
                            (0, 1) => 52,
                            (1, 0) => 54,
                            (1, 1) => 55,
                            _ => continue,
                        };
                        if let Some(peripherals) = &self.esp32c6_peripherals {
                            peripherals
                                .interrupt_matrix
                                .set_source(source as u8, pending);
                        }
                        let Some(interrupt) = self.esp_interrupt_routes.get(&source).copied()
                        else {
                            continue;
                        };
                        let priority = self
                            .esp_interrupt_priorities
                            .get(&interrupt)
                            .copied()
                            .unwrap_or(0);
                        let deliver = pending && priority >= self.esp_interrupt_threshold;
                        if deliver && !esp_timer_was_pending[group][timer] {
                            stats.events = stats.events.saturating_add(1);
                        }
                        esp_timer_was_pending[group][timer] = deliver;
                        if interrupt < 32 {
                            if let Some(plic) = &self.esp_c6_plic {
                                plic.set_line(interrupt as u8, pending);
                            }
                            self.cpu.set_interrupt(interrupt as u16, deliver)?;
                        }
                    }
                }
                // Real ROM and application code program SYSTIMER and the
                // interrupt matrix through MMIO. The device register state is
                // authoritative; the legacy functional-ROM arrays are only a
                // compatibility observation cache.
                let systimer_pending = self
                    .esp32c6_peripherals
                    .as_ref()
                    .map_or([false; 3], |peripherals| {
                        peripherals.systimer.pending(self.now)
                    });
                let previous_systimer_raw = self.esp_systimer_raw;
                self.esp_systimer_raw = systimer_pending
                    .iter()
                    .enumerate()
                    .fold(0_u8, |raw, (alarm, pending)| {
                        raw | (u8::from(*pending) << alarm)
                    });
                for (alarm, pending) in systimer_pending.into_iter().enumerate() {
                    let source = 57 + alarm as u32;
                    if let Some(peripherals) = &self.esp32c6_peripherals {
                        peripherals
                            .interrupt_matrix
                            .set_source(source as u8, pending);
                    }
                    let Some(interrupt) = self
                        .esp32c6_peripherals
                        .as_ref()
                        .and_then(|peripherals| peripherals.interrupt_matrix.route(source as u8))
                        .map(u32::from)
                        .or_else(|| self.esp_interrupt_routes.get(&source).copied())
                    else {
                        continue;
                    };
                    let priority = if interrupt < 32 {
                        self.bus
                            .read(
                                u64::from(0x2000_1010_u32 + interrupt * 4),
                                remu_core::AccessWidth::Word,
                                remu_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(MachineError::Bus)? as u32
                    } else {
                        0
                    };
                    let threshold = self
                        .bus
                        .read(
                            0x2000_1090,
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32;
                    let deliver = pending && priority >= threshold;
                    if deliver && previous_systimer_raw & (1 << alarm) == 0 {
                        stats.events = stats.events.saturating_add(1);
                    }
                    if interrupt < 32 {
                        if let Some(plic) = &self.esp_c6_plic {
                            plic.set_line(interrupt as u8, pending);
                        }
                        self.cpu.set_interrupt(interrupt as u16, deliver)?;
                    }
                }
                if let Some(usb) = &self.esp_usb_serial_jtag
                    && let Some(interrupt) = self.esp_interrupt_routes.get(&48).copied()
                {
                    if let Some(peripherals) = &self.esp32c6_peripherals {
                        peripherals
                            .interrupt_matrix
                            .set_source(48, usb.interrupt_pending());
                    }
                    let priority = if interrupt < 32 {
                        self.bus
                            .read(
                                u64::from(0x2000_1010_u32 + interrupt * 4),
                                remu_core::AccessWidth::Word,
                                remu_core::AccessKind::Read,
                                self.now,
                            )
                            .map_err(MachineError::Bus)? as u32
                    } else {
                        0
                    };
                    let threshold = self
                        .bus
                        .read(
                            0x2000_1090,
                            remu_core::AccessWidth::Word,
                            remu_core::AccessKind::Read,
                            self.now,
                        )
                        .map_err(MachineError::Bus)? as u32;
                    let deliver = usb.interrupt_pending() && priority >= threshold;
                    if deliver && !esp_usb_was_pending {
                        stats.events = stats.events.saturating_add(1);
                    }
                    esp_usb_was_pending = deliver;
                    if interrupt < 32 {
                        if let Some(plic) = &self.esp_c6_plic {
                            plic.set_line(interrupt as u8, usb.interrupt_pending());
                        }
                        self.cpu.set_interrupt(interrupt as u16, deliver)?;
                    }
                }
                let clint = self
                    .esp_c6_clint
                    .as_ref()
                    .map_or([false; 4], |clint| clint.pending(self.now));
                if let (Some(peripherals), Some(plic)) =
                    (&self.esp32c6_peripherals, &self.esp_c6_plic)
                {
                    let pending = peripherals.interrupt_matrix.pending_cpu_interrupts();
                    plic.set_lines(pending);
                }
                let plic_machine = self
                    .esp_c6_plic
                    .as_ref()
                    .map_or(0, |plic| plic.deliverable(false));
                let plic_user = self
                    .esp_c6_plic
                    .as_ref()
                    .map_or(0, |plic| plic.deliverable(true));
                let local = u32::from(clint[2])
                    | (u32::from(clint[0]) << 3)
                    | (u32::from(clint[3]) << 4)
                    | (u32::from(clint[1]) << 7);
                self.cpu
                    .set_interrupt_mask(local | plic_machine | plic_user);
            }
            if watchpoints_active {
                self.bus.clear_watchpoint_hit();
            }
            self.select_rp2350_access_context(0);
            let service_possible = self.target != TargetId::Esp32c6
                || Self::esp32c6_functional_service_address(self.cpu.pc());
            if service_possible {
                self.bus
                    .set_observation_pc(Some(u64::from(self.cpu.pc())));
                let service_result = self.service_functional_bootrom();
                self.bus.set_observation_pc(None);
                match service_result {
                    Ok(true) => {
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(remu_core::SimDuration::TICK)
                            .map_err(|_| MachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(address) = self.bus.take_device_access() {
                            native_peripherals_active |= address < TEST_GPIO;
                            pio_runtime_dirty |= self.pio_runtime_access(address);
                        }
                        if watchpoints_active && let Some(hit) = self.bus.take_watchpoint_hit() {
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                        continue;
                    }
                    Ok(false) => {}
                    Err(message) => break StopReason::Fault(message),
                }
            }
            let instruction_pc = self.cpu.pc();
            self.bus
                .set_observation_pc(Some(u64::from(instruction_pc)));
            let step_result = self.cpu.step(&mut self.bus, self.now);
            self.bus.set_observation_pc(None);
            let outcome = match step_result {
                Ok(outcome) => outcome,
                Err(error) => {
                    break StopReason::Fault(format!(
                        "RISC-V CPU fault at PC {instruction_pc:#010x}: {error}"
                    ));
                }
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| MachineError::TimeOverflow)?;
            stats.time = self.now;
            if let Some(address) = self.bus.take_device_access() {
                native_peripherals_active |= address < TEST_GPIO;
                pio_runtime_dirty |= self.pio_runtime_access(address);
            }
            if native_peripherals_active && let Some(peripherals) = &self.esp32c6_peripherals {
                stats.events = stats
                    .events
                    .saturating_add(peripherals.poll_outputs(self.now)?);
            }

            if self.signals.has_changes() {
            if let Some(path) =
                control.record_signals(&self.signals, &self.signal_stops, &mut trace)?
            {
                    break StopReason::Signal(path);
                }
            }
            if watchpoints_active && let Some(hit) = self.bus.take_watchpoint_hit() {
                break StopReason::Watchpoint {
                    address: hit.address,
                    access: hit.kind,
                };
            }

            match outcome.reason {
                StepReason::Advanced | StepReason::WaitForInterrupt => {}
                StepReason::Halted => break StopReason::Halted,
                StepReason::Breakpoint => break StopReason::Breakpoint,
            }

            if let Some(launch) = self.sio.as_ref().and_then(RpSioHandle::take_core1_launch) {
                if let Err(error) = self.cpu1.set_trap_vector(launch.vector_table) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self
                    .cpu1
                    .set_register(RiscVRegister::Sp, launch.stack_pointer)
                {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_register(RiscVRegister::Ra, 0x80) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_pc(launch.entry) {
                    break StopReason::Fault(format!("RISC-V hart 1 launch: {error}"));
                }
                self.cpu1_active = true;
                stats.events = stats.events.saturating_add(1);
            }
            if self.cpu1_active {
                if let Some(sio) = &self.sio {
                    sio.select_core(1);
                }
                self.select_rp2350_access_context(1);
                if breakpoints_active && self.breakpoints.contains(&u64::from(self.cpu1.pc())) {
                    if let Some(sio) = &self.sio {
                        sio.select_core(0);
                    }
                    break StopReason::Breakpoint;
                }
                self.bus.clear_watchpoint_hit();
                self.bus
                    .set_observation_pc(Some(u64::from(self.cpu1.pc())));
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                let hart1_rom = self.service_functional_bootrom();
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                self.bus.set_observation_pc(None);
                match hart1_rom {
                    Ok(true) => {
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(remu_core::SimDuration::TICK)
                            .map_err(|_| MachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            if let Some(sio) = &self.sio {
                                sio.select_core(0);
                            }
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                    }
                    Ok(false) => {
                        let instruction_pc = self.cpu1.pc();
                        self.bus
                            .set_observation_pc(Some(u64::from(instruction_pc)));
                        let hart1_step = self.cpu1.step(&mut self.bus, self.now);
                        self.bus.set_observation_pc(None);
                        let hart1_outcome = match hart1_step {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                if let Some(sio) = &self.sio {
                                    sio.select_core(0);
                                }
                                break StopReason::Fault(format!(
                                    "RISC-V hart 1 fault at PC {instruction_pc:#010x}: {error}"
                                ));
                            }
                        };
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(hart1_outcome.elapsed)
                            .map_err(|_| MachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            if let Some(sio) = &self.sio {
                                sio.select_core(0);
                            }
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                        match hart1_outcome.reason {
                            StepReason::Advanced | StepReason::WaitForInterrupt => {}
                            StepReason::Halted => self.cpu1_active = false,
                            StepReason::Breakpoint => {
                                if let Some(sio) = &self.sio {
                                    sio.select_core(0);
                                }
                                break StopReason::Breakpoint;
                            }
                        }
                    }
                    Err(message) => {
                        if let Some(sio) = &self.sio {
                            sio.select_core(0);
                        }
                        break StopReason::Fault(format!("RISC-V hart 1 ROM: {message}"));
                    }
                }
                if let Some(address) = self.bus.take_device_access() {
                    native_peripherals_active |= address < TEST_GPIO;
                    pio_runtime_dirty |= self.pio_runtime_access(address);
                }
                if let Some(sio) = &self.sio {
                    sio.select_core(0);
                }
                if let Some(path) =
                    control.record_signals(&self.signals, &self.signal_stops, &mut trace)?
                {
                    break StopReason::Signal(path);
                }
            }
        };

        if let Some(sink) = trace {
            sink.finish()?;
        }
        Ok(RunResult {
            target: self.target,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: self.cpu1_active.then(|| self.cpu1.snapshot()),
            exit_code: self.exit.code(),
            uart: {
                let mut bytes = self.uart.bytes();
                for uart in &self.chip_uarts {
                    bytes.extend(uart.bytes());
                }
                bytes
            },
            usb: self.esp_usb_serial_jtag.as_ref().map_or_else(
                || {
                    self.usb_host
                        .as_ref()
                        .map_or_else(Vec::new, Rp2040UsbHost::output)
                },
                |usb| {
                    let mut bytes = usb.output();
                    bytes.extend(usb.low_speed_output());
                    bytes
                },
            ),
            trace_digest: control.digest.finish(),
        })
    }
}
