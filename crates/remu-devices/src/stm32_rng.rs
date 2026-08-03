//! STM32L4 deterministic RNG register subset.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimDuration, SimTime};
use std::sync::{Arc, Mutex};

const CR: u64 = 0x00;
const SR: u64 = 0x04;
const DR: u64 = 0x08;
const RNGEN: u32 = 1 << 2;
const IE: u32 = 1 << 3;
const DRDY: u32 = 1;
const CECS: u32 = 1 << 1;
const SECS: u32 = 1 << 2;
const CEIS: u32 = 1 << 5;
const SEIS: u32 = 1 << 6;

#[derive(Default)]
struct RngState {
    control: u32,
    seed: u32,
    last: u32,
    ready_at: Option<SimTime>,
    clock_error: bool,
    seed_error: bool,
}

impl RngState {
    fn next(&mut self) -> u32 {
        let mut value = self.seed;
        if value == 0 {
            value = 0x6d2b_79f5;
        }
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.seed = value;
        self.last = value;
        value
    }
}

/// Host-facing deterministic RNG state.
#[derive(Clone)]
pub struct Stm32RngHandle(Arc<Mutex<RngState>>);

impl Stm32RngHandle {
    /// Seeds the deterministic stream used by RNG_DR.
    pub fn seed(&self, seed: u32) {
        let mut state = self.0.lock().expect("STM32 RNG lock poisoned");
        state.seed = seed;
        state.last = 0;
    }

    /// Returns the last generated value.
    pub fn last(&self) -> u32 {
        self.0.lock().expect("STM32 RNG lock poisoned").last
    }
}

/// Functional STM32L432 RNG register block.
pub struct Stm32Rng {
    name: String,
    state: Arc<Mutex<RngState>>,
}

impl Stm32Rng {
    /// Creates a disabled, deterministic RNG.
    pub fn new(name: impl Into<String>) -> (Self, Stm32RngHandle) {
        let state = Arc::new(Mutex::new(RngState {
            seed: 0x6d2b_79f5,
            ..RngState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32RngHandle(state),
        )
    }
}

impl Device for Stm32Rng {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 RNG requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 RNG lock poisoned");
        let value = match offset {
            CR => state.control,
            SR => {
                let mut status = 0;
                if state.control & RNGEN != 0
                    && state.ready_at.is_some_and(|ready_at| ready_at <= _at)
                {
                    status |= DRDY;
                }
                if state.clock_error {
                    status |= CECS | CEIS;
                }
                if state.seed_error {
                    status |= SECS | SEIS;
                }
                status
            }
            DR => {
                if state.control & RNGEN == 0 {
                    return Err(DeviceError::new("STM32 RNG data read while disabled"));
                }
                if !state.ready_at.is_some_and(|ready_at| ready_at <= _at) {
                    return Err(DeviceError::new("STM32 RNG data read while not ready"));
                }
                let value = state.next();
                // The real peripheral clears DRDY when DR is read and refills
                // its conditioning FIFO asynchronously. One abstract tick is
                // enough to preserve that observable ordering without adding
                // cycle-level timing to the functional model.
                state.ready_at = _at.checked_add(SimDuration::TICK).ok();
                value
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 RNG read at {offset:#x}"
                )));
            }
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 RNG requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 RNG lock poisoned");
        match offset {
            CR => {
                let previous = state.control;
                state.control = value as u32 & (RNGEN | IE);
                match (previous & RNGEN != 0, state.control & RNGEN != 0) {
                    (false, true) => state.ready_at = Some(_at),
                    (true, false) => state.ready_at = None,
                    _ => {}
                }
            }
            SR => {
                let value = value as u32;
                // CEIS and SEIS are the only writable SR bits. They are
                // cleared by writing zero; DRDY and the current-status bits
                // are read-only on STM32L4.
                if value & CEIS == 0 {
                    state.clock_error = false;
                }
                if value & SEIS == 0 {
                    state.seed_error = false;
                }
            }
            DR => return Err(DeviceError::new("STM32 RNG data register is read-only")),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 RNG write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 RNG lock poisoned") = RngState {
            seed: 0x6d2b_79f5,
            ..RngState::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_rng_reports_ready_and_replays_seeded_values() {
        let (mut rng, handle) = Stm32Rng::new("rng");
        rng.write(CR, AccessWidth::Word, RNGEN.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(rng.read(SR, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        handle.seed(0x1234_5678);
        let first = rng.read(DR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32;
        assert_eq!(rng.read(SR, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        handle.seed(0x1234_5678);
        let replay = rng
            .read(DR, AccessWidth::Word, SimTime::from_ticks(1))
            .unwrap() as u32;
        assert_eq!(first, replay);
        assert_eq!(handle.last(), first);
    }

    #[test]
    fn status_error_bits_are_read_only_and_clearable() {
        let (mut rng, _) = Stm32Rng::new("rng");
        rng.write(
            SR,
            AccessWidth::Word,
            (CECS | SECS | CEIS | SEIS).into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(rng.read(SR, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }

    #[test]
    fn disabled_rng_data_access_is_rejected() {
        let (mut rng, _) = Stm32Rng::new("rng");
        assert!(rng.read(DR, AccessWidth::Word, SimTime::ZERO).is_err());
    }
}
