use super::*;

const GENERATOR_COUNT: usize = 6;
const CTRL_ENABLE: u32 = 1;
const CTRL_RUNNING: u32 = 1 << 1;
const CYCLES_MASK: u32 = 0x1ff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TickRegisterField {
    Ctrl,
    Cycles,
    Count,
}

/// Named RP2350 TICKS register identifiers.
///
/// The six hardware generators expose the same three registers at consecutive
/// offsets. Keeping the identifiers explicit prevents callers from having to
/// duplicate the register map's magic offsets when inspecting a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rp2350TicksRegister {
    /// Processor 0 control register (offset `0x00`).
    Proc0Ctrl,
    /// Processor 0 period register (offset `0x04`).
    Proc0Cycles,
    /// Processor 0 countdown register (offset `0x08`).
    Proc0Count,
    /// Processor 1 control register (offset `0x0c`).
    Proc1Ctrl,
    /// Processor 1 period register (offset `0x10`).
    Proc1Cycles,
    /// Processor 1 countdown register (offset `0x14`).
    Proc1Count,
    /// Timer 0 control register (offset `0x18`).
    Timer0Ctrl,
    /// Timer 0 period register (offset `0x1c`).
    Timer0Cycles,
    /// Timer 0 countdown register (offset `0x20`).
    Timer0Count,
    /// Timer 1 control register (offset `0x24`).
    Timer1Ctrl,
    /// Timer 1 period register (offset `0x28`).
    Timer1Cycles,
    /// Timer 1 countdown register (offset `0x2c`).
    Timer1Count,
    /// Watchdog control register (offset `0x30`).
    WatchdogCtrl,
    /// Watchdog period register (offset `0x34`).
    WatchdogCycles,
    /// Watchdog countdown register (offset `0x38`).
    WatchdogCount,
    /// RISC-V processor control register (offset `0x3c`).
    RiscvCtrl,
    /// RISC-V processor period register (offset `0x40`).
    RiscvCycles,
    /// RISC-V processor countdown register (offset `0x44`).
    RiscvCount,
}

impl TryFrom<u64> for Rp2350TicksRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        let register = offset & 0x0fff;
        let decoded = match register {
            0x00 => Self::Proc0Ctrl,
            0x04 => Self::Proc0Cycles,
            0x08 => Self::Proc0Count,
            0x0c => Self::Proc1Ctrl,
            0x10 => Self::Proc1Cycles,
            0x14 => Self::Proc1Count,
            0x18 => Self::Timer0Ctrl,
            0x1c => Self::Timer0Cycles,
            0x20 => Self::Timer0Count,
            0x24 => Self::Timer1Ctrl,
            0x28 => Self::Timer1Cycles,
            0x2c => Self::Timer1Count,
            0x30 => Self::WatchdogCtrl,
            0x34 => Self::WatchdogCycles,
            0x38 => Self::WatchdogCount,
            0x3c => Self::RiscvCtrl,
            0x40 => Self::RiscvCycles,
            0x44 => Self::RiscvCount,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 TICKS register at offset {register:#x}"
                )));
            }
        };
        Ok(decoded)
    }
}

impl Rp2350TicksRegister {
    fn generator(self) -> usize {
        match self {
            Self::Proc0Ctrl | Self::Proc0Cycles | Self::Proc0Count => 0,
            Self::Proc1Ctrl | Self::Proc1Cycles | Self::Proc1Count => 1,
            Self::Timer0Ctrl | Self::Timer0Cycles | Self::Timer0Count => 2,
            Self::Timer1Ctrl | Self::Timer1Cycles | Self::Timer1Count => 3,
            Self::WatchdogCtrl | Self::WatchdogCycles | Self::WatchdogCount => 4,
            Self::RiscvCtrl | Self::RiscvCycles | Self::RiscvCount => 5,
        }
    }

    fn field(self) -> TickRegisterField {
        match self {
            Self::Proc0Ctrl
            | Self::Proc1Ctrl
            | Self::Timer0Ctrl
            | Self::Timer1Ctrl
            | Self::WatchdogCtrl
            | Self::RiscvCtrl => TickRegisterField::Ctrl,
            Self::Proc0Cycles
            | Self::Proc1Cycles
            | Self::Timer0Cycles
            | Self::Timer1Cycles
            | Self::WatchdogCycles
            | Self::RiscvCycles => TickRegisterField::Cycles,
            Self::Proc0Count
            | Self::Proc1Count
            | Self::Timer0Count
            | Self::Timer1Count
            | Self::WatchdogCount
            | Self::RiscvCount => TickRegisterField::Count,
        }
    }
}

#[derive(Clone, Copy)]
struct TickGenerator {
    enabled: bool,
    cycles: u16,
    started: SimTime,
}

impl TickGenerator {
    const RESET: Self = Self {
        enabled: false,
        cycles: 0,
        started: SimTime::ZERO,
    };

    fn control(self) -> u32 {
        if self.enabled {
            CTRL_ENABLE | CTRL_RUNNING
        } else {
            0
        }
    }

    fn count(self, at: SimTime) -> u32 {
        if !self.enabled || self.cycles == 0 {
            return 0;
        }
        let period = u64::from(self.cycles);
        let elapsed = at.ticks().saturating_sub(self.started.ticks());
        let remainder = elapsed % period;
        (period - remainder) as u32
    }

    fn apply(&mut self, field: TickRegisterField, alias: u64, value: u32, at: SimTime) {
        match field {
            TickRegisterField::Ctrl => {
                let current = self.control();
                let updated = atomic_update(current, alias, value) & CTRL_ENABLE;
                let enabled = updated & CTRL_ENABLE != 0;
                if enabled != self.enabled {
                    self.started = at;
                }
                self.enabled = enabled;
            }
            TickRegisterField::Cycles => {
                let current = u32::from(self.cycles);
                let updated = atomic_update(current, alias, value) & CYCLES_MASK;
                self.cycles = updated as u16;
                self.started = at;
            }
            TickRegisterField::Count => unreachable!("read-only tick register cannot be written"),
        }
    }
}

fn atomic_update(current: u32, alias: u64, value: u32) -> u32 {
    match alias {
        0 => value,
        1 => current ^ value,
        2 => current | value,
        3 => current & !value,
        _ => unreachable!("atomic alias is limited to two bits"),
    }
}

/// RP2350's six independent hardware tick generators.
///
/// Each generator supplies a deterministic abstract tick source to one of the
/// processor, timer, watchdog, or RISC-V destinations. The model exposes the
/// documented control, period, and countdown registers. It intentionally does
/// not inject interrupts: timer and watchdog devices consume the same abstract
/// simulation timeline and remain independently observable.
pub struct Rp2350Ticks {
    name: String,
    generators: [TickGenerator; GENERATOR_COUNT],
}

impl Rp2350Ticks {
    /// Creates all generators in their reset state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            generators: [TickGenerator::RESET; GENERATOR_COUNT],
        }
    }

    /// Returns whether a generator is enabled at the requested time.
    pub fn is_running(&self, generator: usize, _at: SimTime) -> Option<bool> {
        self.generators.get(generator).map(|tick| tick.enabled)
    }

    /// Returns the remaining countdown for a generator.
    pub fn countdown(&self, generator: usize, at: SimTime) -> Option<u32> {
        self.generators.get(generator).map(|tick| tick.count(at))
    }
}

impl Device for Rp2350Ticks {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 TICKS requires aligned word access",
            ));
        }
        let register = Rp2350TicksRegister::try_from(offset)?;
        let tick = self.generators[register.generator()];
        let value = match register.field() {
            TickRegisterField::Ctrl => tick.control(),
            TickRegisterField::Cycles => u32::from(tick.cycles),
            TickRegisterField::Count => tick.count(at),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 TICKS requires aligned word access",
            ));
        }
        let register = Rp2350TicksRegister::try_from(offset)?;
        if register.field() == TickRegisterField::Count {
            return Err(DeviceError::new("RP2350 TICKS COUNT is read-only"));
        }
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("tick value fits");
        self.generators[register.generator()].apply(register.field(), alias, value, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.generators = [TickGenerator::RESET; GENERATOR_COUNT];
    }
}
