use crate::{
    PinStimulus, RunResult, SignalEdge, SignalStop, TargetId, matching_signal_stop,
    resolve_signal_stop,
};
use remu_bus::{
    AddressSpace, BusAccessRecord, Endianness, Permissions, SharedBusAccessObserver, SharedMemory,
};
use remu_core::{
    AccessKind, AccessWidth, Bus, Cpu, ResetKind, RunLimits, RunStats, SimTime, StepReason,
    StopReason,
};
use remu_cpu_msp430::{Msp430Cpu, Msp430Register};
use remu_devices::{
    GpioHandle, MSP430_PORT1_VECTOR, MSP430_TIMER0_A0_VECTOR, MSP430_USCI_A0_VECTOR,
    MSP430_USCI_B0_VECTOR, Msp430Peripherals, Msp430PeripheralsHandle, SignalHub,
};
use remu_image::{FirmwareArchitecture, FirmwareImage};
use remu_signals::Logic;
use remu_trace::{TraceDigest, TraceSink};
use std::collections::BTreeSet;
use thiserror::Error;

const FRAM_START: u64 = 0xc000;
const FRAM_SIZE: usize = 16 * 1024;

/// MSP430FR2433 machine construction, loading, and execution error.
#[derive(Debug, Error)]
pub enum Msp430MachineError {
    /// This machine only supports the exact FR2433 target.
    #[error("unsupported MSP430 machine target {0}")]
    UnsupportedTarget(TargetId),
    /// Firmware architecture is not MSP430X.
    #[error("firmware for {target} has architecture {actual:?}, expected MSP430X")]
    Architecture {
        /// Selected target.
        target: TargetId,
        /// Parsed ELF architecture.
        actual: FirmwareArchitecture,
    },
    /// Firmware loading failed.
    #[error("failed to load firmware at {address:#x}: {message}")]
    Load {
        /// Guest address.
        address: u64,
        /// Bus diagnostic.
        message: String,
    },
    /// No deterministic execution bound was supplied.
    #[error("MSP430 execution requires an instruction or time limit")]
    MissingRunLimit,
    /// Simulation time overflowed.
    #[error("MSP430 simulation time overflow")]
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

/// Direct-ELF MSP430FR2433 machine with unified FRAM and exact peripheral addresses.
pub struct Msp430McuMachine {
    cpu: Msp430Cpu,
    bus: AddressSpace,
    fram: SharedMemory,
    signals: SignalHub,
    gpio: [GpioHandle; 3],
    peripherals: Msp430PeripheralsHandle,
    now: SimTime,
    breakpoints: BTreeSet<u64>,
    signal_stops: Vec<SignalStop>,
}

impl Msp430McuMachine {
    /// Constructs the exact MSP430FR2433 memory map and peripheral slice.
    pub fn new(target: TargetId) -> Result<Self, Msp430MachineError> {
        if target != TargetId::Msp430fr2433 {
            return Err(Msp430MachineError::UnsupportedTarget(target));
        }
        let signals = SignalHub::new();
        let (peripheral_device, peripherals, gpio) =
            Msp430Peripherals::new("msp430fr2433.peripherals", signals.clone())?;
        let fram = SharedMemory::zeroed(FRAM_SIZE);
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_device(
            "msp430fr2433.peripherals",
            0,
            0x1000,
            Box::new(peripheral_device),
        )?;
        bus.map_ram("msp430fr2433.sram", 0x2000, 4 * 1024, true)?;
        bus.map_shared(
            "msp430fr2433.fram",
            FRAM_START,
            FRAM_SIZE,
            Permissions::RWX,
            fram.clone(),
            0,
        )?;
        Ok(Self {
            cpu: Msp430Cpu::new(),
            bus,
            fram,
            signals,
            gpio,
            peripherals,
            now: SimTime::ZERO,
            breakpoints: BTreeSet::new(),
            signal_stops: Vec::new(),
        })
    }

    /// Loads MSP430 ELF segments and enters the reset vector at `0xfffe`.
    pub fn load_firmware(&mut self, image: &FirmwareImage) -> Result<(), Msp430MachineError> {
        if image.architecture != FirmwareArchitecture::Msp430X {
            return Err(Msp430MachineError::Architecture {
                target: TargetId::Msp430fr2433,
                actual: image.architecture,
            });
        }
        for segment in &image.segments {
            if let Some(load_address) = segment
                .load_address
                .filter(|load_address| *load_address != segment.address)
            {
                self.bus
                    .load(load_address, &segment.data)
                    .map_err(|error| Msp430MachineError::Load {
                        address: load_address,
                        message: error.to_string(),
                    })?;
            }
            self.bus
                .load(segment.address, &segment.data)
                .map_err(|error| Msp430MachineError::Load {
                    address: segment.address,
                    message: error.to_string(),
                })?;
        }
        self.cpu.reset(ResetKind::PowerOn, &mut self.bus)?;
        self.now = SimTime::ZERO;
        Ok(())
    }

    /// Applies a reset without erasing persistent FRAM.
    pub fn reset(&mut self, kind: ResetKind) -> Result<(), Msp430MachineError> {
        self.bus.reset_devices(kind);
        self.cpu.reset(kind, &mut self.bus)?;
        self.now = SimTime::ZERO;
        Ok(())
    }

    /// Returns an immutable copy of all 16 KiB of FRAM for persistence assertions.
    pub fn fram_snapshot(&self) -> Vec<u8> {
        self.fram.to_vec()
    }

    /// Enables or disables completed bus-access recording.
    pub fn set_access_recording(&mut self, enabled: bool) {
        self.bus.set_access_recording(enabled);
    }

    /// Installs or removes a streaming completed-access observer.
    pub fn set_access_observer(&mut self, observer: Option<SharedBusAccessObserver>) {
        self.bus.set_access_observer(observer);
    }

    /// Completed unified-space accesses retained for diagnostics.
    pub fn access_log(&self) -> &[BusAccessRecord] {
        self.bus.access_log()
    }

    /// Adds an execution breakpoint.
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

    /// Adds a unified-space watchpoint.
    pub fn add_watchpoint(&mut self, address: u64) {
        self.bus.add_watchpoint(address);
    }

    /// Stops on a named signal edge.
    pub fn add_signal_stop(
        &mut self,
        path: &str,
        edge: SignalEdge,
    ) -> Result<(), Msp430MachineError> {
        self.signal_stops
            .push(resolve_signal_stop(&self.signals, path, edge)?);
        Ok(())
    }

    /// Drives or releases a package GPIO numbered P1.0..P1.7, P2.0..P2.7, P3.0..P3.2.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), Msp430MachineError> {
        let (port, local_pin) = match pin {
            0..=7 => (0, pin),
            8..=15 => (1, pin - 8),
            16..=18 => (2, pin - 16),
            _ => {
                return Err(remu_bus::DeviceError::new(format!(
                    "MSP430FR2433 package GPIO index {pin} is outside P1.0..P3.2"
                ))
                .into());
            }
        };
        self.gpio[port].set_input(local_pin, value, self.now)?;
        Ok(())
    }

    /// Current Port 1 output latch.
    pub fn gpio_output(&self) -> u32 {
        self.gpio[0].output()
    }

    /// Reads guest-visible bytes from the unified address space.
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

    /// Writes guest-visible bytes to the unified address space.
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
    ) -> Result<RunResult, Msp430MachineError> {
        self.run_with_stimuli(limits, &[], trace)
    }

    /// Runs with timestamped package-pin stimuli.
    pub fn run_with_stimuli(
        &mut self,
        limits: RunLimits,
        stimuli: &[PinStimulus],
        mut trace: Option<&mut dyn TraceSink>,
    ) -> Result<RunResult, Msp430MachineError> {
        if limits.instructions.is_none() && limits.deadline.is_none() {
            return Err(Msp430MachineError::MissingRunLimit);
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
                self.reset(ResetKind::Watchdog)?;
                stats.events = stats.events.saturating_add(1);
            }
            let pending_vectors = self.peripherals.poll(self.now);
            for vector in [
                MSP430_PORT1_VECTOR,
                MSP430_USCI_A0_VECTOR,
                MSP430_USCI_B0_VECTOR,
                MSP430_TIMER0_A0_VECTOR,
            ] {
                self.cpu
                    .set_interrupt(vector, pending_vectors.contains(&vector))?;
            }
            self.bus.clear_watchpoint_hit();
            let outcome = match self.cpu.step(&mut self.bus, self.now) {
                Ok(outcome) => outcome,
                Err(error) => break StopReason::Fault(error.to_string()),
            };
            stats.instructions = stats.instructions.saturating_add(1);
            self.now = self
                .now
                .checked_add(outcome.elapsed)
                .map_err(|_| Msp430MachineError::TimeOverflow)?;
            stats.time = self.now;
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
            target: TargetId::Msp430fr2433,
            reason,
            stats,
            cpu: self.cpu.snapshot(),
            secondary_cpu: None,
            exit_code: Some(self.cpu.register(Msp430Register::R12)),
            uart: self.peripherals.uart_bytes(),
            usb: Vec::new(),
            trace_digest: digest.finish(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_image::FirmwareSegment;

    #[test]
    fn reset_vector_executes_named_exit_register_and_halt() {
        let mut fram = vec![0; FRAM_SIZE];
        let code_offset = 0x0400;
        // mov #0x2a, r12; .word 0
        fram[code_offset..code_offset + 6].copy_from_slice(&[0x3c, 0x40, 0x2a, 0x00, 0x00, 0x00]);
        fram[FRAM_SIZE - 2..].copy_from_slice(&0xc400_u16.to_le_bytes());
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::Msp430X,
            entry: 0xc400,
            segments: vec![FirmwareSegment {
                address: FRAM_START,
                load_address: None,
                initialized_size: fram.len(),
                data: fram,
                executable: true,
                writable: true,
                alignment: 2,
            }],
            symbols: Vec::new(),
        };
        let mut machine = Msp430McuMachine::new(TargetId::Msp430fr2433).unwrap();
        machine.load_firmware(&image).unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(8),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted);
        assert_eq!(result.exit_code, Some(42));
    }

    #[test]
    fn fram_survives_watchdog_reset() {
        let mut machine = Msp430McuMachine::new(TargetId::Msp430fr2433).unwrap();
        machine.debug_write_memory(0xc123, &[0x5a]).unwrap();
        // Supply a valid reset vector before asking the CPU to reset.
        machine.debug_write_memory(0xfffe, &[0x00, 0xc4]).unwrap();
        machine.reset(ResetKind::Watchdog).unwrap();
        assert_eq!(machine.debug_read_memory(0xc123, 1).unwrap(), [0x5a]);
    }
}
