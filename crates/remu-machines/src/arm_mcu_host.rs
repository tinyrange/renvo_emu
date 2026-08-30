use super::*;

impl ArmMcuMachine {
    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.bus.set_access_observer(observer);
    }

    /// Returns completed bus accesses retained for diagnostics.
    pub fn access_log(&self) -> &[BusAccessRecord] {
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

    /// Returns the current architectural snapshot.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Stops after a completed CPU data access overlaps `address`.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Returns the machine's shared signal hub for board endpoint attachment.
    pub fn signal_hub(&self) -> SignalHub {
        self.signals.clone()
    }

    /// Stops when a named signal satisfies an edge condition.
    pub fn add_signal_stop(&mut self, path: &str, edge: SignalEdge) -> Result<(), ArmMachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Drives or releases one package GPIO pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), ArmMachineError> {
        self.gpio.set_input(pin, value, self.now)?;
        if usize::from(pin) < self.compiler_gpio.pin_count() {
            self.compiler_gpio.set_input(pin, value, self.now)?;
        }
        Ok(())
    }

    /// Supplies one deterministic host-side sample to the selected target ADC.
    ///
    /// Guest firmware still controls conversion start through its target's
    /// native registers; this only models the external analog source without
    /// introducing host-dependent voltages or timing.
    pub fn set_adc_sample(&self, channel: u8, value: u16) -> Result<(), ArmMachineError> {
        if let Some(adc) = &self.adc {
            adc.inject_sample(channel, value)?;
            return Ok(());
        }
        if let Some(ra) = &self.ra {
            ra.adc
                .set_input(channel, value)
                .map_err(ArmMachineError::Configuration)?;
            return Ok(());
        }
        Err(ArmMachineError::UnsupportedTarget(self.target))
    }

    /// Supplies one deterministic host-side analog code to the ATSAMD21 AC.
    pub fn set_ac_input(&self, input: u8, value: u16) -> Result<(), ArmMachineError> {
        let Some(ac) = &self.ac else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        ac.inject_input(input, value)?;
        Ok(())
    }

    /// Current vendor GPIO output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio.output()
    }

    /// Returns the host-facing STM32 ADC1 sample handle.
    pub fn adc(&self) -> Option<Stm32AdcHandle> {
        self.stm32_adc.clone()
    }

    /// Returns the host-facing STM32 CRC state.
    pub fn crc(&self) -> Option<Stm32CrcHandle> {
        self.stm32_crc.clone()
    }

    /// Returns the host-facing STM32 RTC state.
    pub fn rtc(&self) -> Option<Stm32RtcHandle> {
        self.stm32_rtc.clone()
    }

    /// Returns the host-facing STM32 RNG state.
    pub fn rng(&self) -> Option<Stm32RngHandle> {
        self.stm32_rng.clone()
    }

    /// Loads bytes into the STM32L432 external QUADSPI flash window.
    pub fn qspi_load_flash(&self, offset: usize, bytes: &[u8]) -> Result<(), ArmMachineError> {
        let Some(qspi) = &self.qspi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        if qspi.load_flash(offset, bytes) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("QUADSPI flash range is out of bounds").into())
        }
    }

    /// Returns a copy of the STM32L432 external QUADSPI flash.
    pub fn qspi_flash(&self) -> Option<Vec<u8>> {
        self.qspi.as_ref().map(Stm32QuadSpiHandle::flash)
    }

    /// Injects one STM32L432 SWPMI receive frame.
    pub fn inject_swpmi_rx(&self, word: u32, frame_bytes: u8) -> Result<(), ArmMachineError> {
        let Some(swpmi) = &self.swpmi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        swpmi.inject_rx(word, frame_bytes, self.now);
        Ok(())
    }

    /// Takes words transmitted by the STM32L432 SWPMI endpoint.
    pub fn take_swpmi_tx(&self) -> Result<Vec<u32>, ArmMachineError> {
        let Some(swpmi) = &self.swpmi else {
            return Err(ArmMachineError::UnsupportedTarget(self.target));
        };
        Ok(swpmi.take_tx())
    }

    /// Supplies a deterministic touch-acquisition count to the STM32 TSC host.
    pub fn set_stm32_tsc_group_count(
        &self,
        group: usize,
        count: u32,
    ) -> Result<(), ArmMachineError> {
        let Some(tsc) = &self.tsc else {
            return Err(
                remu_bus::DeviceError::new("STM32 TSC is not available on this target").into(),
            );
        };
        if tsc.set_group_count(group, count) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("STM32 TSC group index is outside 0..7").into())
        }
    }

    /// Supplies host-side input levels to one STM32 comparator.
    pub fn set_stm32_comparator_inputs(
        &self,
        comparator: usize,
        plus: u16,
        minus: u16,
    ) -> Result<(), ArmMachineError> {
        let Some(comparators) = &self.comparators else {
            return Err(remu_bus::DeviceError::new(
                "STM32 comparators are not available on this target",
            )
            .into());
        };
        if comparators.set_inputs(comparator, plus, minus) {
            Ok(())
        } else {
            Err(remu_bus::DeviceError::new("STM32 comparator index is outside 0..2").into())
        }
    }

    /// Supplies host-side input levels to the STM32 OPAMP.
    pub fn set_stm32_opamp_inputs(&self, plus: u16, minus: u16) -> Result<(), ArmMachineError> {
        let Some(opamp) = &self.opamp else {
            return Err(
                remu_bus::DeviceError::new("STM32 OPAMP is not available on this target").into(),
            );
        };
        opamp.set_inputs(plus, minus);
        Ok(())
    }

    /// Reads guest-visible bytes for qualification and debugger adapters.
    pub fn debug_read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        (0..length)
            .map(|offset| {
                self.bus
                    .read(
                        address + offset as u64,
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map(|value| value as u8)
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    /// Writes guest-visible bytes for debugger adapters.
    pub fn debug_write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    address.saturating_add(offset as u64),
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Services one deterministic transfer unit on each active STM32 DMA channel.
    ///
    /// Normal `run` calls invoke this automatically. The explicit helper is
    /// useful for host-driven peripheral tests that do not need to boot a
    /// firmware image merely to exercise a memory transfer.
    pub fn service_stm32_dma(&mut self) -> Result<usize, ArmMachineError> {
        let mut serviced: usize = 0;
        if let Some(dma) = &self.dma1 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        if let Some(dma) = &self.dma2 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        if let Some(dma) = &self.stm32h7_dma1 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        if let Some(dma) = &self.stm32h7_dma2 {
            serviced = serviced.saturating_add(dma.service(&mut self.bus, self.now)?);
        }
        Ok(serviced)
    }
}
