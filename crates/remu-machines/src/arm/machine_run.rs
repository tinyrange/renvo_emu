use super::*;

const RP2040_IO_BANK0_IRQ: u16 = 13;
const RP2040_SPI0_IRQ: u16 = 18;

impl ArmMachine {
    /// Runs until a limit, exit, breakpoint, or fault.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, ArmMachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped external GPIO stimulus.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, ArmMachineError> {
        if !limits.is_bounded() {
            return Err(ArmMachineError::MissingRunLimit);
        }
        let mut control = RunControl::new(limits, stimuli);
        control.begin_trace(&self.signals, &mut trace)?;
        let mut stats = RunStats {
            instructions: 0,
            time: self.now,
            events: 0,
        };
        let mut timer_was_pending = false;
        let mut chip_timer_was_pending = 0_u16;
        let mut rp2040_io_bank_was_pending = false;
        let mut rp2350_io_bank_was_pending = false;
        let mut trng_was_pending = false;
        let mut rtc_was_pending = false;
        let reason = loop {
            self.sio.select_core(0);
            control.apply_stimuli(self.now, &mut stats, |stimulus| {
                self.set_pin(stimulus.pin, stimulus.value)
            })?;
            if self.exit.code().is_some() {
                break StopReason::Halted;
            }
            if let Some(reason) = control.limit_reason(self.now, &stats) {
                break reason;
            }
            self.refresh_pio_dma_requests()?;
            let accessctrl = self.accessctrl.clone();
            stats.events = stats.events.saturating_add(self.dma.service_with_context(
                &mut self.bus,
                self.now,
                move |_, secure, privileged| {
                    if let Some(accessctrl) = &accessctrl {
                        accessctrl.set_context(Rp2350AccessMaster::Dma, secure, privileged);
                    }
                },
            )? as u64);
            let (dma_irq_base, dma_irq_count) = if self.target == TargetId::Rp2350 {
                (10_u16, 4_usize)
            } else {
                (11_u16, 2_usize)
            };
            for interrupt in 0..dma_irq_count {
                self.cpu.set_interrupt(
                    dma_irq_base + u16::try_from(interrupt).expect("RP DMA IRQ index fits u16"),
                    self.dma.interrupt_pending(interrupt),
                )?;
            }
            if self
                .watchdog
                .as_ref()
                .is_some_and(|watchdog| watchdog.take_reset(self.now))
            {
                break StopReason::Fault("RP2040 watchdog reset".to_owned());
            }
            if self.breakpoints.contains(&self.cpu.snapshot().pc) {
                break StopReason::Breakpoint;
            }
            let timer_pending = self.timer.poll(self.now);
            if timer_pending && !timer_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            timer_was_pending = timer_pending;
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
            if let Some(io_bank) = &self.rp2040_io_bank {
                let pending = io_bank.proc0_pending();
                if pending && !rp2040_io_bank_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                rp2040_io_bank_was_pending = pending;
                self.cpu.set_interrupt(
                    RP2040_IO_BANK0_IRQ,
                    pending && self.ppb.interrupt_enabled(RP2040_IO_BANK0_IRQ),
                )?;
            }
            if let Some(io_bank) = &self.rp2350_io_bank {
                let pending = io_bank.poll(self.now)?;
                if pending && !rp2350_io_bank_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                rp2350_io_bank_was_pending = pending;
                self.cpu.set_interrupt(21, pending)?;
            }
            if let Some(trng) = &self.trng {
                let pending = trng.interrupt_pending() && self.ppb.interrupt_enabled(39);
                if pending && !trng_was_pending {
                    stats.events = stats.events.saturating_add(1);
                }
                trng_was_pending = pending;
                self.cpu.set_interrupt(39, pending)?;
            }
            for (index, handle) in self.i2c.iter().enumerate() {
                self.cpu.set_interrupt(
                    36_u16 + u16::try_from(index).expect("RP2350 I²C index fits u16"),
                    handle.pending(),
                )?;
            }
            for pio in &self.pio {
                if pio.poll(self.now)? {
                    stats.events = stats.events.saturating_add(1);
                }
            }
            self.refresh_pio_dma_requests()?;
            let rtc_pending = self.rtc.as_ref().is_some_and(|rtc| rtc.pending(self.now));
            if rtc_pending && !rtc_was_pending {
                stats.events = stats.events.saturating_add(1);
            }
            rtc_was_pending = rtc_pending;
            self.cpu
                .set_interrupt(0, timer_pending || chip_timer_pending & 1 != 0)?;
            for line in 1..self.chip_timers.len() * 4 {
                self.cpu.set_interrupt(
                    u16::try_from(line).expect("RP timer IRQ line fits u16"),
                    chip_timer_pending & (1 << line) != 0,
                )?;
            }
            self.cpu.set_interrupt(25, rtc_pending)?;
            if let Some(usb) = &self.usb {
                if let (Some(host), Some(dpram)) = (&mut self.usb_host, &self.usb_dpram) {
                    stats.events = stats.events.saturating_add(host.poll(self.now, usb, dpram));
                    if self.stop_on_usb_input_complete && host.input_complete() {
                        break StopReason::HostInputComplete;
                    }
                }
                let usb_irq: u8 = if self.target == TargetId::Rp2040 {
                    5
                } else {
                    14
                };
                self.cpu.set_interrupt(
                    u16::from(usb_irq),
                    usb.interrupt_pending() && self.ppb.interrupt_enabled(u16::from(usb_irq)),
                )?;
            }
            if self.target == TargetId::Rp2040 {
                for (index, spi) in self.chip_spis.iter().enumerate() {
                    let line = RP2040_SPI0_IRQ
                        + u16::try_from(index).expect("RP2040 SPI index fits IRQ line");
                    self.cpu.set_interrupt(
                        line,
                        spi.interrupt_pending() && self.ppb.interrupt_enabled(line),
                    )?;
                }
            }
            for (index, spi) in self.spi.iter().enumerate() {
                let line = 31_u16 + u16::try_from(index).expect("RP2350 SPI index fits IRQ line");
                self.cpu.set_interrupt(
                    line,
                    spi.interrupt_pending() && self.ppb.interrupt_enabled(line),
                )?;
            }
            if self.ppb.take_systick_pending(self.now) {
                self.cpu.set_systick_interrupt(true);
            }
            for line in self.ppb.take_pending_interrupts() {
                self.cpu.set_interrupt(line, true)?;
            }
            let vector_base = self.ppb.vector_base();
            if vector_base != 0 {
                self.cpu.set_vector_base(vector_base);
            }
            self.bus.clear_watchpoint_hit();
            self.select_rp2350_access_context(0);
            match self.service_functional_bootrom() {
                Ok(true) => {
                    stats.instructions = stats.instructions.saturating_add(1);
                    self.now = self
                        .now
                        .checked_add(remu_core::SimDuration::TICK)
                        .map_err(|_| ArmMachineError::TimeOverflow)?;
                    stats.time = self.now;
                    if let Some(hit) = self.bus.take_watchpoint_hit() {
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
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => break StopReason::Fault(error.to_string()),
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| ArmMachineError::TimeOverflow)?;
            stats.time = self.now;
            if let Some(path) =
                control.record_signals(&self.signals, &self.signal_stops, &mut trace)?
            {
                break StopReason::Signal(path);
            }
            if let Some(hit) = self.bus.take_watchpoint_hit() {
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

            if let Some(launch) = self.sio.take_core1_launch() {
                self.cpu1.set_vector_base(launch.vector_table);
                if let Err(error) = self
                    .cpu1
                    .set_direct_state(launch.stack_pointer, launch.entry)
                {
                    break StopReason::Fault(format!("core 1 launch: {error}"));
                }
                if let Err(error) = self.cpu1.set_link_register(0x81) {
                    break StopReason::Fault(format!("core 1 launch: {error}"));
                }
                self.cpu1_active = true;
                stats.events = stats.events.saturating_add(1);
            }
            if self.cpu1_active {
                self.sio.select_core(1);
                self.select_rp2350_access_context(1);
                if self.breakpoints.contains(&self.cpu1.snapshot().pc) {
                    self.sio.select_core(0);
                    break StopReason::Breakpoint;
                }
                self.bus.clear_watchpoint_hit();
                // ROM services are shared between both processors. Temporarily
                // place core 1 in the primary slot so the same architectural
                // service implementation can complete its host call.
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                let core1_rom = self.service_functional_bootrom();
                std::mem::swap(&mut self.cpu, &mut self.cpu1);
                match core1_rom {
                    Ok(true) => {
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(remu_core::SimDuration::TICK)
                            .map_err(|_| ArmMachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            self.sio.select_core(0);
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                    }
                    Ok(false) => {
                        let core1_outcome = match self.cpu1.step(&mut self.bus, self.now) {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                self.sio.select_core(0);
                                break StopReason::Fault(format!("core 1: {error}"));
                            }
                        };
                        stats.instructions = stats.instructions.saturating_add(1);
                        self.now = self
                            .now
                            .checked_add(core1_outcome.elapsed)
                            .map_err(|_| ArmMachineError::TimeOverflow)?;
                        stats.time = self.now;
                        if let Some(hit) = self.bus.take_watchpoint_hit() {
                            self.sio.select_core(0);
                            break StopReason::Watchpoint {
                                address: hit.address,
                                access: hit.kind,
                            };
                        }
                        match core1_outcome.reason {
                            StepReason::Advanced | StepReason::WaitForInterrupt => {}
                            StepReason::Halted => {
                                self.cpu1_active = false;
                            }
                            StepReason::Breakpoint => {
                                self.sio.select_core(0);
                                break StopReason::Breakpoint;
                            }
                        }
                    }
                    Err(message) => {
                        self.sio.select_core(0);
                        break StopReason::Fault(format!("core 1 ROM: {message}"));
                    }
                }
                self.sio.select_core(0);
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
                bytes.extend(self.chip_uart.bytes());
                bytes.extend(self.chip_uart1.bytes());
                bytes
            },
            usb: self
                .usb_host
                .as_ref()
                .map_or_else(Vec::new, Rp2040UsbHost::output),
            trace_digest: control.digest.finish(),
        })
    }
}
