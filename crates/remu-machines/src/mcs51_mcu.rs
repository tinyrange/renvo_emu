use crate::{
    PinStimulus, RunResult, SignalEdge, SignalStop, TargetId, matching_signal_stop,
    resolve_signal_stop,
};
use remu_bus::{AddressSpace, BusAccessRecord, Endianness, SharedBusAccessObserver};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, EventQueue, ResetKind, RunLimits, RunStats, SimTime,
    StepReason, StopReason,
};
use remu_cpu_mcs51::{Mcs51Cpu, Mcs51Register};
use remu_devices::{Efm8Peripherals, Efm8PeripheralsHandle, GpioHandle, SignalHub};
use remu_image::IntelHexImage;
use remu_signals::Logic;
use remu_trace::{TraceDigest, TraceSink};
use std::collections::BTreeSet;
use thiserror::Error;

const SFR_BUS_BASE: u64 = 0x1_0000;
const SFR_BUS_BYTES: usize = 0x1_0000;
const XRAM_BYTES: usize = 2304;

/// EFM8BB52F32G machine construction, loading, and execution error.
#[derive(Debug, Error)]
pub enum Mcs51MachineError {
    /// This machine only supports the exact EFM8 target.
    #[error("unsupported MCS-51 machine target {0}")]
    UnsupportedTarget(TargetId),
    /// A code segment is outside the 32 KiB implementation.
    #[error("EFM8 code segment at {address:#x} with {bytes} bytes exceeds 32 KiB")]
    CodeRange {
        /// First code byte address.
        address: u32,
        /// Segment byte length.
        bytes: usize,
    },
    /// No deterministic execution bound was supplied.
    #[error("EFM8 execution requires an instruction or time limit")]
    MissingRunLimit,
    /// Simulation time overflowed.
    #[error("EFM8 simulation time overflow")]
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
    /// Timestamped stimulus could not be inserted into the stable event queue.
    #[error(transparent)]
    Queue(#[from] remu_core::QueueError),
}

/// Byte-code EFM8BB52F32G machine with the selected functional peripheral slice.
pub struct Mcs51McuMachine {
    cpu: Mcs51Cpu,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: [GpioHandle; 4],
    peripherals: Efm8PeripheralsHandle,
    now: SimTime,
    record_accesses: bool,
    execution_log: Vec<BusAccessRecord>,
    access_observer: Option<SharedBusAccessObserver>,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl Mcs51McuMachine {
    /// Constructs the exact EFM8BB52F32G XRAM, SFR, and package-pin surface.
    pub fn new(target: TargetId) -> Result<Self, Mcs51MachineError> {
        if target != TargetId::Efm8bb52f32g {
            return Err(Mcs51MachineError::UnsupportedTarget(target));
        }
        let signals = SignalHub::new();
        let (device, peripherals, gpio) =
            Efm8Peripherals::new("efm8bb52f32g.sfr", signals.clone())?;
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("efm8bb52f32g.xram", 0, XRAM_BYTES, false)?;
        bus.map_device(
            "efm8bb52f32g.sfr",
            SFR_BUS_BASE,
            SFR_BUS_BYTES,
            Box::new(device),
        )?;
        Ok(Self {
            cpu: Mcs51Cpu::new(),
            bus,
            signals,
            gpio,
            peripherals,
            now: SimTime::ZERO,
            record_accesses: false,
            execution_log: Vec::new(),
            access_observer: None,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }

    /// Loads byte-addressed SDCC Intel HEX output into code flash.
    pub fn load_program(&mut self, image: &IntelHexImage) -> Result<(), Mcs51MachineError> {
        for segment in &image.segments {
            let end = u64::from(segment.address) + segment.data.len() as u64;
            if end > 32 * 1024 {
                return Err(Mcs51MachineError::CodeRange {
                    address: segment.address,
                    bytes: segment.data.len(),
                });
            }
            self.cpu.load_code(segment.address as u16, &segment.data)?;
        }
        self.reset(ResetKind::PowerOn)
    }

    /// Applies an architectural reset without erasing code or XRAM.
    pub fn reset(&mut self, kind: ResetKind) -> Result<(), Mcs51MachineError> {
        self.bus.reset_devices(kind);
        self.cpu.reset(kind, &mut self.bus)?;
        self.now = SimTime::ZERO;
        self.execution_log.clear();
        Ok(())
    }

    /// Enables or disables completed XRAM/SFR access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.record_accesses = enabled;
        if !enabled {
            self.execution_log.clear();
        }
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming code/XRAM/SFR access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.access_observer = observer.clone();
        self.bus.set_access_observer(observer);
    }

    /// Completed code, XRAM, and SFR accesses retained for diagnostics.
    pub fn access_log(&self) -> Vec<BusAccessRecord> {
        let mut accesses = self.execution_log.clone();
        accesses.extend_from_slice(self.bus.access_log());
        accesses.sort_by_key(|access| access.at);
        accesses
    }

    /// Adds a byte-addressed code breakpoint.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
    }

    /// Removes a byte-addressed code breakpoint.
    pub fn remove_breakpoint(&mut self, address: u64) {
        self.breakpoints.remove(&address);
    }

    /// Current named-register architectural snapshot.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Adds an XRAM or SFR watchpoint.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Stops on a named signal edge.
    pub fn add_signal_stop(
        &mut self,
        path: &str,
        edge: SignalEdge,
    ) -> Result<(), Mcs51MachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Drives or releases P0.0..P2.7 followed by P3.0..P3.4.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), Mcs51MachineError> {
        let (port, local_pin) = match pin {
            0..=23 => (usize::from(pin / 8), pin % 8),
            24..=28 => (3, pin - 24),
            _ => {
                return Err(remu_bus::DeviceError::new(format!(
                    "EFM8BB52F32G GPIO index {pin} is outside its 29 package GPIOs"
                ))
                .into());
            }
        };
        self.gpio[port].set_input(local_pin, value, self.now)?;
        Ok(())
    }

    /// Supplies the next byte returned by the EFM8 SPI0 master.
    pub fn inject_spi_rx(&self, value: u8) {
        self.peripherals.inject_spi_rx(value);
    }

    /// Current Port 0 output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio[0].output()
    }

    /// Reads guest-visible XRAM or the machine SFR bus window.
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
                    .map(|value| value.to_le_bytes()[0])
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    /// Writes guest-visible XRAM or the machine SFR bus window.
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

    /// Runs without externally scheduled pin stimuli.
    pub fn run(
        &mut self,
        limits: RunLimits,
        trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, Mcs51MachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped package-pin stimuli.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, Mcs51MachineError> {
        if limits.instructions.is_none() && limits.deadline.is_none() {
            return Err(Mcs51MachineError::MissingRunLimit);
        }
        let mut digest = TraceDigest::new();
        self.signals.with_registry(|registry| {
            digest.begin(registry);
            trace
                .as_deref_mut()
                .map_or(Ok(()), |sink| sink.begin(registry))
        })?;
        let mut stats = RunStats {
            instructions: 0,
            time: self.now,
            events: 0,
        };
        let mut stimulus_queue = EventQueue::new();
        for stimulus in stimuli.iter().copied() {
            stimulus_queue.schedule_at(stimulus.at, stimulus)?;
        }
        let reason = loop {
            while stimulus_queue.next_time().is_some_and(|at| at <= self.now) {
                let stimulus = stimulus_queue
                    .pop()
                    .expect("stimulus queue reported a due event")
                    .payload;
                self.set_pin(stimulus.pin, stimulus.value)?;
                stats.events = stats.events.saturating_add(1);
            }
            if limits
                .instructions
                .is_some_and(|limit| stats.instructions >= limit)
            {
                break StopReason::InstructionLimit;
            }
            if limits.deadline.is_some_and(|deadline| self.now >= deadline) {
                break StopReason::TimeLimit;
            }
            if self.breakpoints.contains(&self.cpu.snapshot().pc) {
                break StopReason::Breakpoint;
            }
            if self.peripherals.take_watchdog_reset() {
                self.bus.reset_devices(ResetKind::Watchdog);
                self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
                stats.events = stats.events.saturating_add(1);
            }
            let interrupt_levels = self.peripherals.poll(self.now);
            for (line, asserted) in interrupt_levels.iter().copied().enumerate() {
                self.cpu.set_interrupt(line as u16, asserted)?;
            }
            self.bus.clear_watchpoint_hit();
            if self.record_accesses || self.access_observer.is_some() {
                let pc = self.cpu.snapshot().pc as u16;
                let record = BusAccessRecord {
                    at: self.now,
                    pc: Some(u64::from(pc)),
                    kind: AccessKind::Execute,
                    address: u64::from(pc),
                    width: AccessWidth::Byte,
                    value: u64::from(self.cpu.code_byte(pc).unwrap_or(0xff)),
                    pre_value: None,
                    post_value: None,
                    region: "efm8bb52f32g.code".to_owned(),
                };
                if self.record_accesses {
                    self.execution_log.push(record.clone());
                }
                if let Some(observer) = &self.access_observer {
                    observer.borrow_mut().observe(&record);
                }
            }
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => break StopReason::Fault(error.to_string()),
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| Mcs51MachineError::TimeOverflow)?;
            stats.time = self.now;
            if matches!(self.cpu.last_interrupt_line(), Some(8 | 9)) {
                self.peripherals.acknowledge_timer1_interrupt(self.now);
            }
            let mut signal_stop = None;
            for change in self.signals.drain_changes() {
                signal_stop =
                    signal_stop.or_else(|| matching_signal_stop(&change, &self.signal_stops));
                digest.change(&change);
                if let Some(sink) = trace.as_deref_mut() {
                    sink.change(&change)?;
                }
            }
            if let Some(path) = signal_stop {
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
        let mut uart = self.peripherals.uart_bytes();
        uart.extend(self.peripherals.uart1_bytes());
        Ok(RunResult {
            target: TargetId::Efm8bb52f32g,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: None,
            exit_code: Some(self.cpu.register(Mcs51Register::A) as u32),
            uart,
            usb: Vec::new(),
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{IntelHexImage, Mcs51McuMachine, PinStimulus, RunLimits, StopReason, TargetId};
    use remu_core::SimTime;
    use remu_signals::Logic;

    #[test]
    fn machine_executes_hex_and_drives_gpio_uart_and_vcd_signals() {
        // MOV P0MDOUT,#1; MOV P0,#1; MOV XBR0,#1; MOV XBR2,#40;
        // MOV SBUF0,#'M'; SJMP -2.
        let image =
            IntelHexImage::parse(b":1100000075A40175800175E10175E34075994D80FE17\n:00000001FF\n")
                .unwrap();
        let mut machine = Mcs51McuMachine::new(TargetId::Efm8bb52f32g).unwrap();
        machine.load_program(&image).unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(12),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::InstructionLimit);
        assert_eq!(machine.gpio_output() & 1, 1);
        assert_eq!(result.uart, b"M");
    }

    #[test]
    fn equal_timestamp_stimuli_preserve_input_order() {
        let image = IntelHexImage::parse(b":04000000000000FC00\n:00000001FF\n").unwrap();
        let mut machine = Mcs51McuMachine::new(TargetId::Efm8bb52f32g).unwrap();
        machine.load_program(&image).unwrap();
        let result = machine
            .run_with_stimuli(
                RunLimits {
                    instructions: Some(1),
                    deadline: None,
                },
                &[
                    PinStimulus {
                        at: SimTime::ZERO,
                        pin: 0,
                        value: Logic::One,
                    },
                    PinStimulus {
                        at: SimTime::ZERO,
                        pin: 0,
                        value: Logic::Zero,
                    },
                ],
                None,
            )
            .unwrap();

        assert_eq!(result.stats.events, 2);
        assert_eq!(machine.gpio[0].resolved(0).unwrap(), Logic::Zero);
    }

    #[test]
    fn future_stimulus_waits_across_resumed_runs_until_machine_time_reaches_it() {
        let image = IntelHexImage::parse(b":04000000000000FC00\n:00000001FF\n").unwrap();
        let mut machine = Mcs51McuMachine::new(TargetId::Efm8bb52f32g).unwrap();
        machine.load_program(&image).unwrap();

        let first = machine
            .run(
                RunLimits {
                    instructions: Some(1),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(first.stats.events, 0);

        let second = machine
            .run_with_stimuli(
                RunLimits {
                    instructions: Some(1),
                    deadline: None,
                },
                &[PinStimulus {
                    at: SimTime::from_ticks(2),
                    pin: 0,
                    value: Logic::One,
                }],
                None,
            )
            .unwrap();
        assert_eq!(second.stats.events, 1);
        assert_eq!(machine.gpio[0].resolved(0).unwrap(), Logic::One);
    }

    #[test]
    fn machine_exposes_native_spi0_sfr_transfer() {
        let image = IntelHexImage::parse(b":0100000000FF\n:00000001FF\n").unwrap();
        let mut machine = Mcs51McuMachine::new(TargetId::Efm8bb52f32g).unwrap();
        machine.load_program(&image).unwrap();
        machine.inject_spi_rx(0x5a);
        machine
            .debug_write_memory(0x1_00f8, &[1])
            .expect("SPI0CN0 write should map");
        machine
            .debug_write_memory(0x1_00a3, &[0xa6])
            .expect("SPI0DAT write should map");
        assert_eq!(machine.debug_read_memory(0x1_00a3, 1).unwrap(), [0x5a]);
    }
}
