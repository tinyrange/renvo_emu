use super::*;

fn advance_16bit_timer(
    state: &mut Efm8State,
    now: u64,
    epoch: u64,
    control: usize,
    current_low: usize,
    current_high: usize,
    reload_low: usize,
    reload_high: usize,
    run_bit: u8,
    low_flag: u8,
    high_flag: u8,
) -> u64 {
    if state.registers[control] & run_bit == 0 {
        return epoch;
    }
    let initial = u16::from_le_bytes([state.registers[current_low], state.registers[current_high]]);
    let elapsed = now.saturating_sub(epoch);
    let low_until_overflow = u64::from(0x100_u16 - (initial & 0xff));
    let until_overflow = u64::from(u16::MAX - initial) + 1;
    if elapsed >= until_overflow {
        state.registers[control] |= high_flag | low_flag;
        state.registers[current_low] = state.registers[reload_low];
        state.registers[current_high] = state.registers[reload_high];
    } else {
        if elapsed >= low_until_overflow {
            state.registers[control] |= low_flag;
        }
        let value = initial.wrapping_add((elapsed & u64::from(u16::MAX)) as u16);
        let [low, high] = value.to_le_bytes();
        state.registers[current_low] = low;
        state.registers[current_high] = high;
    }
    now
}

impl Efm8PeripheralsHandle {
    /// Captured UART0 transmit bytes.
    pub fn uart_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").uart.clone()
    }

    /// Captured UART1 transmit bytes.
    pub fn uart1_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").uart1.clone()
    }

    /// Returns the physical pin currently assigned to a crossbar function.
    pub fn crossbar_pin(&self, function: Efm8CrossbarFunction) -> Option<Efm8CrossbarPin> {
        self.0.lock().expect("EFM8 lock poisoned").crossbar_routes[function.index()]
    }

    /// Reports whether the port crossbar output drivers are enabled.
    pub fn crossbar_enabled(&self) -> bool {
        self.0.lock().expect("EFM8 lock poisoned").registers[XBR2] & XBR2_XBARE != 0
    }

    /// Returns the functional SYSCLK source selected by `CLKSEL.CLKSL`.
    pub fn clock_source(&self) -> Efm8ClockSource {
        self.0.lock().expect("EFM8 lock poisoned").clock_source()
    }

    /// Returns the active SYSCLK divider (one of 1, 2, 4, ..., 128).
    pub fn clock_divider(&self) -> u32 {
        self.0.lock().expect("EFM8 lock poisoned").clock_divider()
    }

    /// Returns the nominal functional SYSCLK frequency in hertz.
    pub fn system_clock_hz(&self) -> u32 {
        self.0.lock().expect("EFM8 lock poisoned").system_clock_hz()
    }

    /// Sets the nominal host frequency used when `CLKSEL` selects EXTOSC.
    pub fn set_external_clock_hz(&self, hz: u32, at: SimTime) -> Result<(), DeviceError> {
        if hz == 0 {
            return Err(DeviceError::new(
                "EFM8 external clock frequency must be non-zero",
            ));
        }
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.external_clock_hz = hz;
        state.refresh_clock(at);
        Ok(())
    }

    /// Returns the current functional CPU power mode.
    pub fn power_mode(&self) -> Efm8PowerMode {
        self.0.lock().expect("EFM8 lock poisoned").power_mode
    }

    /// Wakes IDLE or SNOOZE after a host-side interrupt stimulus.
    pub fn wake(&self, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        if matches!(
            state.power_mode,
            Efm8PowerMode::Idle | Efm8PowerMode::Snooze
        ) {
            state.set_power_mode(Efm8PowerMode::Active, at);
        }
    }

    /// Copies an Intel HEX code segment into the functional flash image.
    pub fn load_flash(&self, address: u32, bytes: &[u8]) -> Result<(), DeviceError> {
        self.0.lock().expect("EFM8 lock poisoned").load_flash(
            usize::try_from(address).map_err(|_| {
                DeviceError::new("EFM8 flash image address does not fit host usize")
            })?,
            bytes,
        )
    }

    /// Reads one byte from the functional flash image.
    pub fn flash_read(&self, address: u16) -> Result<u8, DeviceError> {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .flash_read(usize::from(address))
    }

    /// Applies one firmware MOVX flash program or page-erase request.
    pub fn flash_write(&self, address: u16, value: u8) -> Result<(), DeviceError> {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .flash_write(usize::from(address), value)
    }

    /// Reports whether PSWE currently redirects MOVX writes into flash.
    pub fn flash_write_enabled(&self) -> bool {
        self.0.lock().expect("EFM8 lock poisoned").registers[PSCTL] & PSCTL_PSWE != 0
    }

    /// Returns whether a masked port differs from its configured match value.
    pub fn port_match_event(&self) -> bool {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .port_match_active()
    }

    /// Returns the resolved PCA CEX output for a channel.
    pub fn pca_output(&self, channel: usize) -> Logic {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_outputs
            .get(channel)
            .copied()
            .unwrap_or(Logic::X)
    }

    /// Returns the current 16-bit PCA counter.
    pub fn pca_counter(&self) -> u16 {
        self.0.lock().expect("EFM8 lock poisoned").pca_counter()
    }

    /// Supplies a sampled CEX input edge for a capture channel.
    pub fn set_pca_input(
        &self,
        channel: usize,
        value: Logic,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .capture_pca_input(channel, value, at)
    }

    /// Returns the currently asserted PCA interrupt request.
    pub fn pca_interrupt_pending(&self) -> bool {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .pca_interrupt_pending()
    }

    /// Captured SMBus 0 bytes written by the guest to the transmit FIFO.
    pub fn smbus0_tx_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").smbus0_tx.clone()
    }

    /// Returns whether the functional SMBus 0 state machine owns the bus.
    pub fn smbus0_busy(&self) -> bool {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_BUSY != 0
    }

    /// Returns whether SMBus 0 has an enabled service request pending.
    pub fn smbus0_interrupt(&self) -> bool {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[Efm8SmbusRegister::Eie1.offset()] & EIE1_ESMB0 != 0
            && state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_SI != 0
            && (state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] & SMB0CN0_MASTER != 0
                || state.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_INH == 0)
    }

    /// Queues bytes as a deterministic follower-side SMBus 0 receive event.
    pub fn inject_smbus0_rx(&self, bytes: &[u8], at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        if state.registers[Efm8SmbusRegister::Smb0Cf.offset()] & SMB0CF_ENSMB == 0 {
            return;
        }
        state.smbus0_rx.extend(bytes.iter().copied());
        if let Some(&first) = state.smbus0_rx.front() {
            state.registers[Efm8SmbusRegister::Smb0Dat.offset()] = first;
            state.registers[Efm8SmbusRegister::Smb0Cf.offset()] |= SMB0CF_BUSY;
            state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] &=
                !(SMB0CN0_MASTER | SMB0CN0_TXMODE);
            state.registers[Efm8SmbusRegister::Smb0Cn0.offset()] |= SMB0CN0_ACKRQ | SMB0CN0_SI;
        }
        state.update_smbus0_signals(at);
        state.update_interrupt_signals(at);
    }

    /// Supplies one received UART0 byte and raises RI.
    pub fn inject_uart_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[SBUF0] = value;
        state.registers[SCON0] |= SCON0_RI;
        state.update_interrupt_signals(at);
    }

    /// Supplies one received UART1 byte when its receiver and baud generator
    /// are enabled. The bounded FIFO raises the documented overrun flag when full.
    pub fn inject_uart1_rx(&self, value: u8, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        if state.registers[SBCON1] & SBCON1_BREN == 0 || state.registers[SCON1] & SCON1_REN == 0 {
            return;
        }
        if state.uart1_rx.len() >= 16 {
            state.registers[SCON1] |= 0x80;
        } else {
            state.uart1_rx.push_back(value);
            state.uart1_last_rx = value;
            state.registers[SCON1] |= SCON1_RI;
        }
        state.update_interrupt_signals(at);
    }

    /// Sets one deterministic analog input code for the ADC multiplexer.
    pub fn set_adc_input(&self, channel: u8, value: u16) -> Result<(), DeviceError> {
        let channel = usize::from(channel);
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let input = state.adc_inputs.get_mut(channel).ok_or_else(|| {
            DeviceError::new(format!("EFM8 ADC channel {channel} is outside 0..31"))
        })?;
        *input = value.min(0x0fff);
        Ok(())
    }

    /// Supplies deterministic scalar codes to one comparator's inputs.
    pub fn set_comparator_inputs(
        &self,
        comparator: u8,
        positive: u16,
        negative: u16,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let comparator = usize::from(comparator);
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let inputs = state.comparator_inputs.get_mut(comparator).ok_or_else(|| {
            DeviceError::new(format!("EFM8 comparator {comparator} is outside 0..1"))
        })?;
        *inputs = [positive, negative];
        state.refresh_comparators(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    /// Supplies resolved A/B logic values to one configurable logic unit.
    pub fn set_clu_inputs(
        &self,
        clu: u8,
        a: bool,
        b: bool,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let inputs = state
            .clu_input_overrides
            .get_mut(usize::from(clu))
            .ok_or_else(|| DeviceError::new(format!("EFM8 CLU index {clu} is outside 0..3")))?;
        *inputs = Some([a, b]);
        state.refresh_clu(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    /// Releases a CLU host-input override and returns to mux resolution.
    pub fn clear_clu_inputs(&self, clu: u8, at: SimTime) -> Result<(), DeviceError> {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        let inputs = state
            .clu_input_overrides
            .get_mut(usize::from(clu))
            .ok_or_else(|| DeviceError::new(format!("EFM8 CLU index {clu} is outside 0..3")))?;
        *inputs = None;
        state.refresh_clu(at);
        state.update_interrupt_signals(at);
        Ok(())
    }

    /// Returns the current selected output of one configurable logic unit.
    pub fn clu_output(&self, clu: u8) -> Result<bool, DeviceError> {
        let state = self.0.lock().expect("EFM8 lock poisoned");
        let index = usize::from(clu);
        if index >= 4 {
            return Err(DeviceError::new(format!(
                "EFM8 CLU index {clu} is outside 0..3"
            )));
        }
        Ok(state.clu_output(index))
    }

    /// Supplies the next byte returned by a functional SPI0 master transfer.
    pub fn inject_spi_rx(&self, value: u8) {
        self.0
            .lock()
            .expect("EFM8 lock poisoned")
            .spi_rx
            .push(value);
    }

    /// Captured bytes written to SPI0DAT.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("EFM8 lock poisoned").spi_tx.clone()
    }

    /// Applies the native Timer1 side effect of vectoring to its interrupt.
    ///
    /// EFM8 hardware clears TF1 when the core acknowledges the Timer1
    /// interrupt. The machine calls this only after the MCS-51 core has
    /// actually selected the Timer1 vector, so a masked flag remains visible
    /// until it is serviced or explicitly cleared by firmware.
    pub fn acknowledge_timer1_interrupt(&self, at: SimTime) {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        state.registers[TCON] &= !TCON_TF1;
        state.update_interrupt_signals(at);
    }

    /// Advances functional timers/watchdog and returns low/high CPU interrupt inputs.
    pub fn poll(&self, now: SimTime) -> [bool; 32] {
        let mut state = self.0.lock().expect("EFM8 lock poisoned");
        for port in 0..4 {
            let _ = state.refresh_port(port, now);
        }
        if state.registers[TCON] & TCON_TR0 != 0 {
            let initial = u16::from_be_bytes([state.registers[TH0], state.registers[TL0]]);
            let elapsed = now.ticks().saturating_sub(state.timer0_epoch);
            let total = u64::from(initial).saturating_add(elapsed);
            let mode = state.registers[TMOD] & 3;
            if mode == 2 {
                let reload = state.registers[TH0];
                let period = u64::from(256_u16 - u16::from(reload)).max(1);
                state.registers[TL0] = reload.wrapping_add((elapsed % period).to_le_bytes()[0]);
                if elapsed >= period {
                    state.registers[TCON] |= TCON_TF0;
                    state.timer0_epoch = now.ticks();
                }
            } else {
                let bytes = total.to_le_bytes();
                state.registers[TL0] = bytes[0];
                state.registers[TH0] = bytes[1];
                state.timer0_epoch = now.ticks();
                if total > u64::from(u16::MAX) {
                    state.registers[TCON] |= TCON_TF0;
                }
            }
        }
        if state.registers[TCON] & TCON_TR1 != 0 {
            let mode = (state.registers[TMOD] >> 4) & 3;
            let elapsed = now.ticks().saturating_sub(state.timer1_epoch);
            match mode {
                1 => {
                    let initial = u16::from_be_bytes([state.registers[TH1], state.registers[TL1]]);
                    let total = u64::from(initial).saturating_add(elapsed);
                    let [low, high] = (total as u16).to_le_bytes();
                    state.registers[TL1] = low;
                    state.registers[TH1] = high;
                    if total > u64::from(u16::MAX) {
                        state.registers[TCON] |= TCON_TF1;
                    }
                    state.timer1_epoch = now.ticks();
                }
                2 => {
                    // In auto-reload mode the first overflow depends on the
                    // current TL1 value. Subsequent overflows reload TH1.
                    let initial = u64::from(state.registers[TL1]);
                    let total = initial.saturating_add(elapsed);
                    let reload = state.registers[TH1];
                    let period = u64::from(256_u16 - u16::from(reload)).max(1);
                    if total >= 256 {
                        let after_first = total - 256;
                        state.registers[TL1] = reload.wrapping_add((after_first % period) as u8);
                        state.registers[TCON] |= TCON_TF1;
                    } else {
                        state.registers[TL1] = total as u8;
                    }
                    state.timer1_epoch = now.ticks();
                }
                // Mode 0 is the legacy 13-bit form and mode 3 leaves Timer1
                // inactive on the EFM8. Neither mode is part of this
                // functional slice; rebase time so changing modes while the
                // timer is running cannot count the unsupported interval.
                _ => state.timer1_epoch = now.ticks(),
            }
        }
        if state.registers[TMR2CN0] & TMR2_TR2 != 0 {
            let initial = u16::from_le_bytes([state.registers[TMR2L], state.registers[TMR2H]]);
            let elapsed = now.ticks().saturating_sub(state.timer2_epoch);
            let until_overflow = u64::from(u16::MAX - initial) + 1;
            if elapsed >= until_overflow {
                state.registers[TMR2CN0] |= TMR2_TF2H;
                state.registers[TMR2L] = state.registers[TMR2RLL];
                state.registers[TMR2H] = state.registers[TMR2RLH];
                state.timer2_epoch = now.ticks();
            } else {
                let elapsed = u16::try_from(elapsed)
                    .expect("non-overflowing Timer2 elapsed value fits in 16 bits");
                let value = initial.wrapping_add(elapsed);
                let [low, high] = value.to_le_bytes();
                state.registers[TMR2L] = low;
                state.registers[TMR2H] = high;
                state.timer2_epoch = now.ticks();
            }
        }
        let timer3_epoch = state.timer3_epoch;
        state.timer3_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer3_epoch,
            TMR3CN0,
            TMR3L,
            TMR3H,
            TMR3RLL,
            TMR3RLH,
            TMR3_TR3,
            TMR3_TF3L,
            TMR3_TF3H,
        );
        let timer4_epoch = state.timer4_epoch;
        state.timer4_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer4_epoch,
            TMR4CN0,
            TMR4L,
            TMR4H,
            TMR4RLL,
            TMR4RLH,
            TMR4_TR4,
            TMR4_TF4L,
            TMR4_TF4H,
        );
        let timer5_epoch = state.timer5_epoch;
        state.timer5_epoch = advance_16bit_timer(
            &mut state,
            now.ticks(),
            timer5_epoch,
            TMR5CN0,
            TMR5L,
            TMR5H,
            TMR5RLL,
            TMR5RLH,
            TMR5_TR5,
            TMR5_TF5L,
            TMR5_TF5H,
        );
        if state.watchdog_enabled && now.ticks().saturating_sub(state.watchdog_epoch) >= 65_536 {
            state.watchdog_reset = true;
            state.set_signal(state.watchdog_reset_signal, 1, 1, now);
        }
        let _ = state.advance_pca(now);
        state.refresh_clu(now);
        state.refresh_port_match(now);
        state.update_smbus0_signals(now);
        state.update_interrupt_signals(now);
        let levels = state.interrupt_levels();
        if levels.iter().any(|level| *level)
            && matches!(
                state.power_mode,
                Efm8PowerMode::Idle | Efm8PowerMode::Snooze
            )
        {
            state.set_power_mode(Efm8PowerMode::Active, now);
        }
        levels
    }

    /// Consumes a watchdog reset request.
    pub fn take_watchdog_reset(&self) -> bool {
        std::mem::take(&mut self.0.lock().expect("EFM8 lock poisoned").watchdog_reset)
    }
}
