use crate::{
    PinStimulus, RunResult, SignalEdge, SignalStop, TargetId, resolve_signal_stop,
    run_control::RunControl,
};
use remu_bus::{AddressSpace, BusAccessRecord, Endianness, SharedBusAccessObserver};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, ResetKind, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use remu_cpu_avr::{AvrCpu, AvrRegister};
use remu_devices::{AtmegaIo, AtmegaIoHandle, GpioHandle, SignalHub};
use remu_image::{FirmwareArchitecture, FirmwareImage};
use remu_signals::Logic;
use remu_trace::TraceSink;
use std::collections::BTreeSet;
use thiserror::Error;

const ADC_INTERRUPT_LINE: u16 = 20;

/// ATmega machine construction, loading, and execution error.
#[derive(Debug, Error)]
pub enum AvrMachineError {
    /// This machine only supports the exact ATmega328PB target.
    #[error("unsupported AVR machine target {0}")]
    UnsupportedTarget(TargetId),
    /// Firmware architecture is not AVR8.
    #[error("firmware for {target} has architecture {actual:?}, expected AVR8")]
    Architecture {
        /// Selected target.
        target: TargetId,
        /// Parsed ELF architecture.
        actual: FirmwareArchitecture,
    },
    /// Firmware loading failed.
    #[error("failed to load firmware at {address:#x}: {message}")]
    Load {
        /// Translated data-space address.
        address: u64,
        /// Bus diagnostic.
        message: String,
    },
    /// No deterministic execution bound was supplied.
    #[error("AVR execution requires an instruction or time limit")]
    MissingRunLimit,
    /// Simulation time overflowed.
    #[error("AVR simulation time overflow")]
    TimeOverflow,
    /// Bus map construction failed.
    #[error(transparent)]
    Map(#[from] remu_bus::MapError),
    /// Signal construction or lookup failed.
    #[error(transparent)]
    Signal(#[from] remu_signals::SignalError),
    /// CPU loading or execution failed.
    #[error(transparent)]
    Cpu(#[from] remu_core::CpuFault),
    /// Package pin drive failed.
    #[error(transparent)]
    Device(#[from] remu_bus::DeviceError),
    /// Trace output failed.
    #[error(transparent)]
    Trace(#[from] remu_trace::TraceError),
}

/// Direct-ELF ATmega328PB machine with Harvard flash and PB-specific I/O.
pub struct AvrMcuMachine {
    cpu: AvrCpu,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: [GpioHandle; 3],
    io: AtmegaIoHandle,
    now: SimTime,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl AvrMcuMachine {
    /// Constructs the exact ATmega328PB machine.
    pub fn new(target: TargetId) -> Result<Self, AvrMachineError> {
        if target != TargetId::Atmega328pb {
            return Err(AvrMachineError::UnsupportedTarget(target));
        }
        let signals = SignalHub::new();
        let (io_device, io, gpio) = AtmegaIo::new("atmega328pb.io", signals.clone())?;
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_device("atmega328pb.io", 0x20, 0xe0, Box::new(io_device))?;
        bus.map_ram("atmega328pb.sram", 0x100, 2 * 1024, false)?;
        Ok(Self {
            cpu: AvrCpu::new(),
            bus,
            signals,
            gpio,
            io,
            now: SimTime::ZERO,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }

    /// Loads AVR ELF segments, translating the ELF data-space marker at `0x800000`.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), AvrMachineError> {
        if image.architecture != FirmwareArchitecture::Avr8 {
            return Err(AvrMachineError::Architecture {
                target: TargetId::Atmega328pb,
                actual: image.architecture,
            });
        }
        let mut program = vec![0xff; 32 * 1024];
        for segment in &image.segments {
            if let Some(load_address) = segment.load_address.filter(|address| *address < 0x800000) {
                let start = usize::try_from(load_address).map_err(|_| AvrMachineError::Load {
                    address: load_address,
                    message: "flash load address does not fit host usize".to_owned(),
                })?;
                let end =
                    start
                        .checked_add(segment.data.len())
                        .ok_or_else(|| AvrMachineError::Load {
                            address: load_address,
                            message: "flash load segment overflow".to_owned(),
                        })?;
                let destination =
                    program
                        .get_mut(start..end)
                        .ok_or_else(|| AvrMachineError::Load {
                            address: load_address,
                            message: "flash load segment exceeds 32 KiB".to_owned(),
                        })?;
                destination.copy_from_slice(&segment.data);
            }
            if segment.address < 0x800000 {
                let start =
                    usize::try_from(segment.address).map_err(|_| AvrMachineError::Load {
                        address: segment.address,
                        message: "flash address does not fit host usize".to_owned(),
                    })?;
                let end =
                    start
                        .checked_add(segment.data.len())
                        .ok_or_else(|| AvrMachineError::Load {
                            address: segment.address,
                            message: "flash segment overflow".to_owned(),
                        })?;
                let destination =
                    program
                        .get_mut(start..end)
                        .ok_or_else(|| AvrMachineError::Load {
                            address: segment.address,
                            message: "flash segment exceeds 32 KiB".to_owned(),
                        })?;
                destination.copy_from_slice(&segment.data);
            } else {
                let address = segment.address - 0x800000;
                self.bus
                    .load(address, &segment.data)
                    .map_err(|error| AvrMachineError::Load {
                        address,
                        message: error.to_string(),
                    })?;
            }
        }
        self.cpu.load_program(&program)?;
        Ok(())
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }
    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.bus.set_access_observer(observer);
    }
    /// Completed data-space accesses retained for diagnostics.
    pub fn access_log(&self) -> &[BusAccessRecord] {
        self.bus.access_log()
    }
    /// Adds an execution breakpoint at a byte-addressed AVR PC.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
    }
    /// Removes an execution breakpoint.
    pub fn remove_breakpoint(&mut self, address: u64) {
        self.breakpoints.remove(&address);
    }
    /// Current architectural snapshot.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }
    /// Adds a data-space watchpoint.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }
    /// Returns the machine's shared signal hub for board endpoint attachment.
    pub fn signal_hub(&self) -> SignalHub {
        self.signals.clone()
    }

    /// Stops on a named GPIO edge.
    pub fn add_signal_stop(&mut self, path: &str, edge: SignalEdge) -> Result<(), AvrMachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }
    /// Drives or releases a package GPIO numbered B0..B7, C0..C6, D0..D7.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), AvrMachineError> {
        let (port, local_pin) = match pin {
            0..=7 => (0, pin),
            8..=14 => (1, pin - 8),
            15..=22 => (2, pin - 15),
            _ => {
                return Err(remu_bus::DeviceError::new(format!(
                    "ATmega328PB package GPIO index {pin} is outside B0..D7"
                ))
                .into());
            }
        };
        self.gpio[port].set_input(local_pin, value, self.now)?;
        Ok(())
    }
    /// Supplies the next byte returned by the ATmega328PB SPI0 master.
    pub fn inject_spi_rx(&self, value: u8) {
        self.io.inject_spi_rx(value);
    }

    /// Drives the functional ATmega328PB analog-comparator inputs.
    ///
    /// This is a deterministic boolean abstraction of AIN0/AIN1. It does not
    /// model analog voltages, noise, propagation delay, or the bandgap input.
    pub fn set_comparator_inputs(&self, positive: bool, negative: bool) {
        self.io.set_comparator_inputs(positive, negative, self.now);
    }
    /// Current PORTB output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio[0].output()
    }

    /// Drives one deterministic 10-bit ADC channel sample.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        self.io.set_adc_input(channel, value);
    }

    /// Reads guest-visible AVR data-space bytes.
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

    /// Writes guest-visible AVR data-space bytes.
    pub fn debug_write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        for (offset, byte) in bytes.iter().copied().enumerate() {
            self.bus
                .write(
                    address + offset as u64,
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Runs without externally scheduled stimuli.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, AvrMachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped PORTB input stimuli.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, AvrMachineError> {
        if !limits.is_bounded() {
            return Err(AvrMachineError::MissingRunLimit);
        }
        let mut control = RunControl::new(limits, stimuli);
        control.begin_trace(&self.signals, &mut trace)?;
        let mut stats = RunStats {
            instructions: 0,
            time: self.now,
            events: 0,
        };
        let reason = loop {
            control.apply_stimuli(self.now, &mut stats, |stimulus| {
                self.set_pin(stimulus.pin, stimulus.value)
            })?;
            if let Some(reason) = control.limit_reason(self.now, &stats) {
                break reason;
            }
            if self.breakpoints.contains(&self.cpu.snapshot().pc) {
                break StopReason::Breakpoint;
            }
            if self.io.take_watchdog_reset() {
                self.bus.reset_devices(ResetKind::Watchdog);
                self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
                stats.events = stats.events.saturating_add(1);
            }
            let interrupt_lines = self.io.poll(self.now);
            for line in interrupt_lines.iter().copied() {
                self.cpu.set_interrupt(line, true)?;
            }
            // ADC completion is a level derived from ADIF/ADIE. Clear the
            // core's pending input when firmware clears ADIF before vectoring.
            self.cpu.set_interrupt(
                ADC_INTERRUPT_LINE,
                interrupt_lines.contains(&ADC_INTERRUPT_LINE),
            )?;
            self.cpu.set_sleep_enabled(self.io.sleep_enabled());
            self.bus.clear_watchpoint_hit();
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => break StopReason::Fault(error.to_string()),
            };
            stats.instructions = stats.instructions.saturating_add(1);
            let elapsed = outcome
                .elapsed
                .checked_mul(self.io.clock_divider())
                .map_err(|_| AvrMachineError::TimeOverflow)?;
            self.now = self
                .now
                .checked_add(elapsed)
                .map_err(|_| AvrMachineError::TimeOverflow)?;
            stats.time = self.now;
            if self.cpu.last_interrupt_line() == Some(ADC_INTERRUPT_LINE) {
                self.io.acknowledge_adc_interrupt(self.now);
            }
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
        };
        if let Some(sink) = trace {
            sink.finish()?;
        }
        Ok(RunResult {
            target: TargetId::Atmega328pb,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: None,
            exit_code: Some(self.cpu.register(AvrRegister::R24) as u32),
            uart: {
                let mut bytes = self.io.uart_bytes();
                bytes.extend(self.io.uart1_bytes());
                bytes
            },
            usb: Vec::new(),
            trace_digest: control.digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_devices::AtmegaComparatorRegister;
    use remu_image::FirmwareSegment;

    #[test]
    fn atmega_executes_ldi_out_and_break_with_named_exit_register() {
        // ldi r24, 0; ldi r16, 1; out DDRB,r16; out PORTB,r16; break
        let words = [0xe080_u16, 0xe001, 0xb904, 0xb905, 0x9598];
        let code = words
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::Avr8,
            entry: 0,
            segments: vec![FirmwareSegment {
                address: 0,
                load_address: None,
                initialized_size: code.len(),
                data: code,
                executable: true,
                writable: false,
                alignment: 2,
            }],
            symbols: Vec::new(),
        };
        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.load_firmware(&image).unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(20),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(machine.gpio_output(), 1);
    }

    #[test]
    fn atmega_maps_native_spi0_registers() {
        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.inject_spi_rx(0x5a);
        machine.debug_write_memory(0x4c, &[1 << 6]).unwrap();
        machine.debug_write_memory(0x4e, &[0xa6]).unwrap();
        assert_eq!(machine.debug_read_memory(0x4e, 1).unwrap(), [0x5a]);
    }

    #[test]
    fn atmega_exposes_scripted_adc_samples_through_native_registers() {
        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.set_adc_input(2, 0x0155);
        machine
            .bus
            .write(0x7c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x7a, AccessWidth::Byte, 0x88, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x7a, AccessWidth::Byte, 0xc8, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine.io.poll(SimTime::from_ticks(50)),
            vec![ADC_INTERRUPT_LINE]
        );
        assert_eq!(
            machine
                .bus
                .read(0x78, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0x55
        );
    }

    #[test]
    fn atmega_comparator_host_input_updates_acsr() {
        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        // ACSR: ACIE plus rising-output edge mode.
        machine
            .debug_write_memory(u64::from(AtmegaComparatorRegister::Acsr.offset()), &[0x0b])
            .unwrap();
        machine.set_comparator_inputs(true, false);
        let status = machine
            .debug_read_memory(u64::from(AtmegaComparatorRegister::Acsr.offset()), 1)
            .unwrap()[0];
        assert_ne!(status & 0x20, 0, "ACO should reflect AIN0 > AIN1");
        assert_ne!(status & 0x10, 0, "rising output should latch ACI");
    }

    #[test]
    fn atmega_sleep_enable_and_clock_prescaler_are_machine_visible() {
        let words = [0x9588_u16, 0x9598]; // sleep; break
        let code = words
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::Avr8,
            entry: 0,
            segments: vec![FirmwareSegment {
                address: 0,
                load_address: None,
                initialized_size: code.len(),
                data: code,
                executable: true,
                writable: false,
                alignment: 2,
            }],
            symbols: Vec::new(),
        };

        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.load_firmware(&image).unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(4),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted);
        assert!(!result.cpu.waiting);
        assert_eq!(result.stats.instructions, 2);

        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.load_firmware(&image).unwrap();
        machine.debug_write_memory(0x53, &[1]).unwrap(); // SMCR.SE
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(3),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::InstructionLimit);
        assert!(result.cpu.waiting);
        assert_eq!(result.stats.instructions, 3);

        let words = [0x0000_u16, 0x9598]; // nop; break
        let code = words
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::Avr8,
            entry: 0,
            segments: vec![FirmwareSegment {
                address: 0,
                load_address: None,
                initialized_size: code.len(),
                data: code,
                executable: true,
                writable: false,
                alignment: 2,
            }],
            symbols: Vec::new(),
        };
        let mut machine = AvrMcuMachine::new(TargetId::Atmega328pb).unwrap();
        machine.load_firmware(&image).unwrap();
        machine.debug_write_memory(0x61, &[0x80]).unwrap(); // CLKPR: CLKPCE
        machine.debug_write_memory(0x61, &[2]).unwrap(); // divide by four
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(4),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted);
        assert_eq!(result.stats.time, SimTime::from_ticks(8));
    }
}
