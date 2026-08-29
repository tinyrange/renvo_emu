use crate::{
    PinStimulus, RunResult, SignalEdge, SignalStop, TargetId, matching_signal_stop,
    resolve_signal_stop,
};
use remu_bus::{AddressSpace, BusAccessRecord, Endianness, SharedBusAccessObserver};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, ResetKind, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use remu_cpu_pic16::{Pic16Cpu, Pic16Register};
use remu_devices::{GpioHandle, Pic16Peripherals, Pic16PeripheralsHandle, SignalHub};
use remu_image::ProgramWordImage;
use remu_signals::Logic;
use remu_trace::{TraceDigest, TraceSink};
use std::collections::BTreeSet;
use thiserror::Error;

/// PIC16F15376 machine construction, loading, and execution error.
#[derive(Debug, Error)]
pub enum Pic16MachineError {
    /// This machine only supports the exact F15376 target.
    #[error("unsupported PIC16 machine target {0}")]
    UnsupportedTarget(TargetId),
    /// Program image does not contain 14-bit words.
    #[error("PIC16F15376 requires 14-bit program words, image has {0}")]
    ProgramWidth(u8),
    /// A program segment is outside the 16K-word implementation.
    #[error("PIC16 program segment at word {address:#x} with {words} words exceeds 16K words")]
    ProgramRange {
        /// First word address.
        address: u32,
        /// Segment length.
        words: usize,
    },
    /// No deterministic execution bound was supplied.
    #[error("PIC16 execution requires an instruction or time limit")]
    MissingRunLimit,
    /// Simulation time overflowed.
    #[error("PIC16 simulation time overflow")]
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

/// Word-addressed PIC16F15376 machine with the selected functional peripheral slice.
pub struct Pic16McuMachine {
    cpu: Pic16Cpu,
    bus: AddressSpace,
    signals: SignalHub,
    gpio: [GpioHandle; 5],
    peripherals: Pic16PeripheralsHandle,
    now: SimTime,
    record_accesses: bool,
    execution_log: Vec<BusAccessRecord>,
    access_observer: Option<SharedBusAccessObserver>,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl Pic16McuMachine {
    /// Constructs the exact PIC16F15376 data map and peripheral slice.
    pub fn new(target: TargetId) -> Result<Self, Pic16MachineError> {
        if target != TargetId::Pic16f15376 {
            return Err(Pic16MachineError::UnsupportedTarget(target));
        }
        let signals = SignalHub::new();
        let (device, peripherals, gpio) =
            Pic16Peripherals::new("pic16f15376.data", signals.clone())?;
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_device("pic16f15376.data", 0, 0x2000, Box::new(device))?;
        Ok(Self {
            cpu: Pic16Cpu::new(),
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

    /// Loads 14-bit program words reconstructed from XC8 Intel HEX output.
    pub fn load_program(&mut self, image: &ProgramWordImage) -> Result<(), Pic16MachineError> {
        if image.word_bits != 14 {
            return Err(Pic16MachineError::ProgramWidth(image.word_bits));
        }
        for segment in &image.segments {
            let end = segment.address as u64 + segment.words.len() as u64;
            // Configuration words live above normal executable program memory and are
            // retained by the HEX parser but do not participate in this functional slice.
            if segment.address >= 16 * 1024 {
                continue;
            }
            if end > 16 * 1024 {
                return Err(Pic16MachineError::ProgramRange {
                    address: segment.address,
                    words: segment.words.len(),
                });
            }
            let words = segment
                .words
                .iter()
                .map(|word| *word as u16)
                .collect::<Vec<_>>();
            self.cpu
                .load_program_words(segment.address as u16, &words)?;
        }
        self.bus.reset_devices(ResetKind::PowerOn);
        self.cpu.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.now = SimTime::ZERO;
        self.execution_log.clear();
        Ok(())
    }

    /// Applies an architectural reset without erasing program memory.
    pub fn reset(&mut self, kind: ResetKind) -> Result<(), Pic16MachineError> {
        self.bus.reset_devices(kind);
        self.cpu.reset(kind, &mut self.bus)?;
        self.now = SimTime::ZERO;
        self.execution_log.clear();
        Ok(())
    }

    /// Enables or disables completed data-space access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.record_accesses = enabled;
        if !enabled {
            self.execution_log.clear();
        }
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming program/data access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.access_observer = observer.clone();
        self.bus.set_access_observer(observer);
    }

    /// Completed data-space accesses retained for diagnostics.
    pub fn access_log(&self) -> Vec<BusAccessRecord> {
        let mut accesses = self.execution_log.clone();
        accesses.extend_from_slice(self.bus.access_log());
        accesses.sort_by_key(|access| access.at);
        accesses
    }

    /// Adds a word-addressed execution breakpoint.
    pub fn add_breakpoint(&mut self, address: u64) {
        self.breakpoints.insert(address);
    }

    /// Removes a word-addressed execution breakpoint.
    pub fn remove_breakpoint(&mut self, address: u64) {
        self.breakpoints.remove(&address);
    }

    /// Current architectural snapshot.
    pub fn debug_snapshot(&self) -> remu_core::CpuSnapshot {
        self.cpu.snapshot()
    }

    /// Adds a byte data-space watchpoint.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Stops on a named signal edge.
    pub fn add_signal_stop(
        &mut self,
        path: &str,
        edge: SignalEdge,
    ) -> Result<(), Pic16MachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Drives or releases package GPIO A0..D7 followed by E0..E3.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), Pic16MachineError> {
        let (port, local_pin) = match pin {
            0..=31 => (usize::from(pin / 8), pin % 8),
            32..=35 => (4, pin - 32),
            _ => {
                return Err(remu_bus::DeviceError::new(format!(
                    "PIC16F15376 GPIO index {pin} is outside A0..E3"
                ))
                .into());
            }
        };
        self.gpio[port].set_input(local_pin, value, self.now)?;
        Ok(())
    }

    /// Drives a deterministic 10-bit value into one ADC channel.
    pub fn set_adc_input(&self, channel: u8, value: u16) {
        self.peripherals.set_adc_input(channel, value);
    }

    /// Current Port A output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio[0].output()
    }

    /// Reads guest-visible byte data space.
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

    /// Writes guest-visible byte data space.
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
    ) -> Result<RunResult, Pic16MachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped package pin stimuli.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, Pic16MachineError> {
        if !limits.is_bounded() {
            return Err(Pic16MachineError::MissingRunLimit);
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
        let mut stimuli = stimuli.to_vec();
        stimuli.sort_by_key(|stimulus| stimulus.at);
        let mut next_stimulus = 0;
        let reason = loop {
            while stimuli
                .get(next_stimulus)
                .is_some_and(|stimulus| stimulus.at <= self.now)
            {
                let stimulus = stimuli[next_stimulus];
                self.set_pin(stimulus.pin, stimulus.value)?;
                next_stimulus += 1;
                stats.events = stats.events.saturating_add(1);
            }
            if let Some(reason) = limits.reached(stats.instructions, self.now) {
                break reason;
            }
            if self.breakpoints.contains(&self.cpu.snapshot().pc) {
                break StopReason::Breakpoint;
            }
            if self.peripherals.take_watchdog_reset() {
                self.bus.reset_devices(ResetKind::Watchdog);
                self.cpu.reset(ResetKind::Watchdog, &mut self.bus)?;
                stats.events = stats.events.saturating_add(1);
            }
            self.cpu.set_interrupt(0, self.peripherals.poll(self.now))?;
            self.bus.clear_watchpoint_hit();
            if self.record_accesses || self.access_observer.is_some() {
                let pc = self.cpu.snapshot().pc as u16;
                let record = BusAccessRecord {
                    at: self.now,
                    pc: Some(u64::from(pc)),
                    kind: AccessKind::Execute,
                    address: u64::from(pc),
                    width: AccessWidth::HalfWord,
                    value: u64::from(self.cpu.program_word(pc).unwrap_or(0x3fff)),
                    pre_value: None,
                    post_value: None,
                    region: "pic16f15376.program".to_owned(),
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
                .map_err(|_| Pic16MachineError::TimeOverflow)?;
            stats.time = self.now;
            if self.cpu.take_watchdog_clear() {
                self.peripherals.clear_watchdog(self.now);
            }
            if let Some(kind) = self.cpu.take_reset_request() {
                self.bus.reset_devices(kind);
                stats.events = stats.events.saturating_add(1);
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
        Ok(RunResult {
            target: TargetId::Pic16f15376,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: None,
            exit_code: Some(self.cpu.register(Pic16Register::Wreg) as u32),
            uart: self.peripherals.uart_bytes(),
            usb: Vec::new(),
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_image::{ProgramWordEndianness, ProgramWordSegment};

    #[test]
    fn machine_executes_word_image_and_drives_gpio() {
        // MOVLB 62; CLRF ANSELA; MOVLB 0; MOVLW fe; MOVWF TRISA;
        // MOVLW 1; MOVWF LATA; BRA -1.
        let image = ProgramWordImage {
            word_bits: 14,
            endianness: ProgramWordEndianness::Little,
            segments: vec![ProgramWordSegment {
                address: 0,
                words: vec![
                    0x017e, 0x01b8, 0x0140, 0x30fe, 0x0092, 0x3001, 0x0098, 0x33ff,
                ],
            }],
            entry: None,
        };
        let mut machine = Pic16McuMachine::new(TargetId::Pic16f15376).unwrap();
        machine.load_program(&image).unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(20),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::InstructionLimit);
        assert_eq!(machine.gpio_output() & 1, 1);
    }

    #[test]
    fn machine_exposes_pic16_adc_conversion() {
        let image = ProgramWordImage {
            word_bits: 14,
            endianness: ProgramWordEndianness::Little,
            segments: vec![ProgramWordSegment {
                address: 0,
                words: vec![0x0000, 0x0000, 0x0000],
            }],
            entry: None,
        };
        let mut machine = Pic16McuMachine::new(TargetId::Pic16f15376).unwrap();
        machine.load_program(&image).unwrap();
        machine.set_adc_input(2, 0x155);
        machine
            .bus
            .write(0x09e, AccessWidth::Byte, 1 << 7, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x09d, AccessWidth::Byte, (2 << 2) | 0x03, SimTime::ZERO)
            .unwrap();
        machine
            .run(
                RunLimits {
                    instructions: Some(2),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(machine.debug_read_memory(0x09b, 2).unwrap(), [0x55, 0x01]);
    }
}
