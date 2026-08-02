use super::*;

const GENERATOR_COUNT: usize = 6;
const GENERATOR_STRIDE: u64 = 0x0c;
const CTRL: u64 = 0x00;
const CYCLES: u64 = 0x04;
const COUNT: u64 = 0x08;
const CTRL_ENABLE: u32 = 1;
const CTRL_RUNNING: u32 = 1 << 1;
const CYCLES_MASK: u32 = 0x1ff;

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

    fn apply(&mut self, register: u64, alias: u64, value: u32, at: SimTime) {
        match register {
            CTRL => {
                let current = self.control();
                let updated = atomic_update(current, alias, value) & CTRL_ENABLE;
                let enabled = updated & CTRL_ENABLE != 0;
                if enabled != self.enabled {
                    self.started = at;
                }
                self.enabled = enabled;
            }
            CYCLES => {
                let current = u32::from(self.cycles);
                let updated = atomic_update(current, alias, value) & CYCLES_MASK;
                self.cycles = updated as u16;
                self.started = at;
            }
            _ => unreachable!("read-only tick register cannot be written"),
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

    fn register(offset: u64) -> Result<(usize, u64), DeviceError> {
        let register = offset & 0x0fff;
        let generator = usize::try_from(register / GENERATOR_STRIDE).expect("tick index fits");
        let field = register % GENERATOR_STRIDE;
        if generator >= GENERATOR_COUNT || !matches!(field, CTRL | CYCLES | COUNT) {
            return Err(DeviceError::new(format!(
                "unmodeled RP2350 TICKS register at offset {register:#x}"
            )));
        }
        Ok((generator, field))
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
        let (generator, field) = Self::register(offset)?;
        let tick = self.generators[generator];
        let value = match field {
            CTRL => tick.control(),
            CYCLES => u32::from(tick.cycles),
            COUNT => tick.count(at),
            _ => unreachable!(),
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
        let (generator, field) = Self::register(offset)?;
        if field == COUNT {
            return Err(DeviceError::new("RP2350 TICKS COUNT is read-only"));
        }
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("tick value fits");
        self.generators[generator].apply(field, alias, value, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.generators = [TickGenerator::RESET; GENERATOR_COUNT];
    }
}
