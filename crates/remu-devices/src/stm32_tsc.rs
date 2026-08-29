use super::*;

const IO_MASK: u32 = 0x0fff_ffff;
const GROUPS: usize = 7;
const CR_TSCE: u32 = 1 << 0;
const CR_START: u32 = 1 << 1;
const CR_MCV_MASK: u32 = 0x7 << 5;
const CR_SUPPORTED: u32 = CR_TSCE | CR_START | CR_MCV_MASK | (0x7 << 12) | (1 << 15);
const IER_EOAIE: u32 = 1 << 0;
const IER_MCEIE: u32 = 1 << 1;
const ISR_EOAF: u32 = 1 << 0;
const ISR_MCEF: u32 = 1 << 1;

/// Named register identifiers for the STM32L4 touch-sensing controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stm32TscRegister {
    /// Control register.
    Cr,
    /// Interrupt enable register.
    Ier,
    /// Interrupt clear register.
    Icr,
    /// Interrupt status register.
    Isr,
    /// I/O hysteresis control register.
    Iohcr,
    /// I/O analog switch control register.
    Ioascr,
    /// I/O sampling control register.
    Ioscr,
    /// I/O channel control register.
    Ioccr,
    /// I/O group control/status register.
    Iogcsr,
    /// One group acquisition counter.
    Iogxcr(usize),
}

struct Stm32TscState {
    cr: u32,
    ier: u32,
    isr: u32,
    iohcr: u32,
    ioascr: u32,
    ioscr: u32,
    ioccr: u32,
    iogcsr: u32,
    counters: [u32; GROUPS],
    host_counts: [u32; GROUPS],
}

/// Host-facing handle for deterministic TSC acquisition inputs and interrupts.
#[derive(Clone)]
pub struct Stm32TscHandle {
    state: Arc<Mutex<Stm32TscState>>,
}

impl Stm32TscHandle {
    /// Sets the next deterministic acquisition count for one group.
    pub fn set_group_count(&self, group: usize, count: u32) -> bool {
        let mut state = self.state.lock().expect("STM32 TSC lock poisoned");
        let Some(slot) = state.host_counts.get_mut(group) else {
            return false;
        };
        *slot = count;
        true
    }

    /// Starts one functional acquisition using the configured host counts.
    pub fn start(&self) -> bool {
        let mut state = self.state.lock().expect("STM32 TSC lock poisoned");
        if state.cr & CR_TSCE == 0 {
            return false;
        }
        Self::acquire(&mut state);
        true
    }

    /// Returns the most recent count for one group.
    pub fn group_count(&self, group: usize) -> Option<u32> {
        let state = self.state.lock().expect("STM32 TSC lock poisoned");
        state.counters.get(group).copied()
    }

    /// Returns true when an enabled end-of-acquisition or max-count flag is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("STM32 TSC lock poisoned");
        state.isr & state.ier & (ISR_EOAF | ISR_MCEF) != 0
    }

    fn acquire(state: &mut Stm32TscState) {
        let max_count = 1_u32 << (8 + ((state.cr & CR_MCV_MASK) >> 5));
        let enabled_groups = state.iogcsr & 0x7f;
        for group in 0..GROUPS {
            if enabled_groups & (1 << group) == 0 {
                continue;
            }
            let requested = state.host_counts[group];
            if requested >= max_count {
                state.counters[group] = max_count - 1;
                state.isr |= ISR_MCEF;
            } else {
                state.counters[group] = requested;
            }
        }
        state.isr |= ISR_EOAF;
        state.cr &= !CR_START;
    }
}

/// Functional STM32L432KC touch-sensing controller.
///
/// The model preserves the documented register layout and acquisition flags.
/// Electrical charge-transfer waveforms are represented by deterministic host
/// supplied group counts, which is suitable for firmware and compiler tests
/// without claiming analog or clock-level fidelity.
pub struct Stm32Tsc {
    name: String,
    state: Arc<Mutex<Stm32TscState>>,
}

impl Stm32Tsc {
    /// Creates a reset controller and host handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32TscHandle) {
        let state = Arc::new(Mutex::new(Stm32TscState {
            cr: 0,
            ier: 0,
            isr: 0,
            iohcr: 0,
            ioascr: 0,
            ioscr: 0,
            ioccr: 0,
            iogcsr: 0,
            counters: [0; GROUPS],
            host_counts: [128; GROUPS],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32TscHandle { state },
        )
    }

    fn decode(offset: u64) -> Option<Stm32TscRegister> {
        Some(match offset {
            0x00 => Stm32TscRegister::Cr,
            0x04 => Stm32TscRegister::Ier,
            0x08 => Stm32TscRegister::Icr,
            0x0c => Stm32TscRegister::Isr,
            0x10 => Stm32TscRegister::Iohcr,
            0x18 => Stm32TscRegister::Ioascr,
            0x20 => Stm32TscRegister::Ioscr,
            0x28 => Stm32TscRegister::Ioccr,
            0x30 => Stm32TscRegister::Iogcsr,
            offset if (0x34..=0x4c).contains(&offset) && offset % 4 == 0 => {
                Stm32TscRegister::Iogxcr(usize::try_from((offset - 0x34) / 4).ok()?)
            }
            _ => return None,
        })
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 TSC requires aligned word accesses"));
        }
        Ok(())
    }
}

impl Device for Stm32Tsc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let state = self.state.lock().expect("STM32 TSC lock poisoned");
        let value = match register {
            Stm32TscRegister::Cr => state.cr,
            Stm32TscRegister::Ier => state.ier,
            Stm32TscRegister::Icr => 0,
            Stm32TscRegister::Isr => state.isr,
            Stm32TscRegister::Iohcr => state.iohcr,
            Stm32TscRegister::Ioascr => state.ioascr,
            Stm32TscRegister::Ioscr => state.ioscr,
            Stm32TscRegister::Ioccr => state.ioccr,
            Stm32TscRegister::Iogcsr => state.iogcsr,
            Stm32TscRegister::Iogxcr(group) => state.counters[group],
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("STM32 TSC lock poisoned");
        match register {
            Stm32TscRegister::Cr => {
                let previous = state.cr;
                state.cr = value & CR_SUPPORTED;
                if state.cr & CR_START != 0 && previous & CR_START == 0 {
                    Stm32TscHandle::acquire(&mut state);
                }
            }
            Stm32TscRegister::Ier => state.ier = value & (IER_EOAIE | IER_MCEIE),
            Stm32TscRegister::Icr => state.isr &= !(value & (ISR_EOAF | ISR_MCEF)),
            Stm32TscRegister::Isr => {
                return Err(DeviceError::new(format!(
                    "{} ISR is read-only; use ICR to clear flags",
                    self.name
                )));
            }
            Stm32TscRegister::Iohcr => state.iohcr = value & IO_MASK,
            Stm32TscRegister::Ioascr => state.ioascr = value & IO_MASK,
            Stm32TscRegister::Ioscr => state.ioscr = value & IO_MASK,
            Stm32TscRegister::Ioccr => state.ioccr = value & IO_MASK,
            Stm32TscRegister::Iogcsr => {
                state.iogcsr = (state.iogcsr & 0x007f_0000) | (value & 0x7f)
            }
            Stm32TscRegister::Iogxcr(_) => {
                return Err(DeviceError::new(format!(
                    "{} group counters are read-only",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("STM32 TSC lock poisoned");
        state.cr = 0;
        state.ier = 0;
        state.isr = 0;
        state.iohcr = 0;
        state.ioascr = 0;
        state.ioscr = 0;
        state.ioccr = 0;
        state.iogcsr = 0;
        state.counters = [0; GROUPS];
        state.host_counts = [128; GROUPS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};
    use remu_core::{AccessKind, Bus};

    #[test]
    fn host_count_acquisition_sets_counter_and_clearable_irq() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let (tsc, handle) = Stm32Tsc::new("tsc");
        bus.map_device("tsc", 0x4002_4000, 0x100, Box::new(tsc))
            .unwrap();
        assert!(handle.set_group_count(0, 0x321));
        bus.write(0x4002_4030, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_4004,
            AccessWidth::Word,
            u64::from(IER_EOAIE),
            SimTime::ZERO,
        )
        .unwrap();
        bus.write(
            0x4002_4000,
            AccessWidth::Word,
            u64::from(CR_TSCE | CR_START | (3 << 5)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            bus.read(
                0x4002_4034,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x321
        );
        assert!(handle.interrupt_pending());
        bus.write(
            0x4002_4008,
            AccessWidth::Word,
            u64::from(ISR_EOAF),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn max_count_sets_error_flag_and_clamps_counter() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let (tsc, handle) = Stm32Tsc::new("tsc");
        bus.map_device("tsc", 0x4002_4000, 0x100, Box::new(tsc))
            .unwrap();
        handle.set_group_count(1, 300);
        bus.write(0x4002_4030, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_4000,
            AccessWidth::Word,
            u64::from(CR_TSCE | CR_START),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            bus.read(
                0x4002_4038,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            255
        );
        assert_eq!(
            bus.read(
                0x4002_400c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
                & u64::from(ISR_MCEF),
            u64::from(ISR_MCEF)
        );
    }
}
