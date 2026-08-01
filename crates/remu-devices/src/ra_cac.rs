//! RA4M1 Clock Frequency Accuracy Measurement Circuit.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const CACR0: u64 = 0x00;
const CACR1: u64 = 0x01;
const CACR2: u64 = 0x02;
const CAICR: u64 = 0x03;
const CASTR: u64 = 0x04;
const CAULVR: u64 = 0x06;
const CALLVR: u64 = 0x08;
const CACNTBR: u64 = 0x0a;

const CFME: u8 = 1 << 0;
const FERRIE: u8 = 1 << 0;
const MENDIE: u8 = 1 << 1;
const OVFIE: u8 = 1 << 2;
const FERRFCL: u8 = 1 << 4;
const MENDFCL: u8 = 1 << 5;
const OVFFCL: u8 = 1 << 6;

#[derive(Default)]
struct CacState {
    control0: u8,
    control1: u8,
    control2: u8,
    interrupt: u8,
    status: u8,
    upper: u16,
    lower: u16,
    count: u16,
}

impl CacState {
    fn reference_edge(&mut self, count: u16) {
        if self.control0 & CFME == 0 {
            return;
        }
        self.count = count;
        self.status |= 1 << 1; // MENDF
        if count < self.lower || count > self.upper {
            self.status |= 1 << 0; // FERRF
        }
    }
}

/// Host-facing RA4M1 CAC state and deterministic reference-edge injection.
#[derive(Clone)]
pub struct RaCacHandle(Arc<Mutex<CacState>>);

impl RaCacHandle {
    /// Injects one completed reference measurement into the circuit.
    pub fn reference_edge(&self, count: u16) {
        self.0
            .lock()
            .expect("RA CAC lock poisoned")
            .reference_edge(count);
    }

    /// Returns the last captured counter value.
    pub fn count(&self) -> u16 {
        self.0.lock().expect("RA CAC lock poisoned").count
    }

    /// Returns `(frequency_error, measurement_end, overflow)` flags.
    pub fn flags(&self) -> (bool, bool, bool) {
        let state = self.0.lock().expect("RA CAC lock poisoned");
        (
            state.status & 1 != 0,
            state.status & (1 << 1) != 0,
            state.status & (1 << 2) != 0,
        )
    }
}

/// Functional RA4M1 CAC register and measurement subset.
pub struct RaCac {
    name: String,
    state: Arc<Mutex<CacState>>,
}

impl RaCac {
    /// Creates a reset-state CAC block.
    pub fn new(name: impl Into<String>) -> (Self, RaCacHandle) {
        let state = Arc::new(Mutex::new(CacState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaCacHandle(state),
        )
    }
}

impl Device for RaCac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("RA CAC lock poisoned");
        match offset {
            CACR0 | CACR1 | CACR2 | CAICR | CASTR => {
                if width != AccessWidth::Byte {
                    return Err(DeviceError::new(
                        "RA CAC control/status requires byte access",
                    ));
                }
                let value = match offset {
                    CACR0 => state.control0,
                    CACR1 => state.control1,
                    CACR2 => state.control2,
                    CAICR => state.interrupt & 0x07,
                    CASTR => state.status & 0x07,
                    _ => unreachable!(),
                };
                Ok(u64::from(value))
            }
            CAULVR => {
                if width != AccessWidth::HalfWord {
                    return Err(DeviceError::new("RA CAC limits require half-word access"));
                }
                Ok(u64::from(state.upper))
            }
            CALLVR => {
                if width != AccessWidth::HalfWord {
                    return Err(DeviceError::new("RA CAC limits require half-word access"));
                }
                Ok(u64::from(state.lower))
            }
            CACNTBR => {
                if width != AccessWidth::HalfWord {
                    return Err(DeviceError::new("RA CAC counter requires half-word access"));
                }
                Ok(u64::from(state.count))
            }
            _ => Err(DeviceError::new(format!(
                "unmodeled RA CAC read at {offset:#x}"
            ))),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RA CAC lock poisoned");
        match offset {
            CACR0 | CACR1 | CACR2 | CAICR | CASTR => {
                if width != AccessWidth::Byte {
                    return Err(DeviceError::new(
                        "RA CAC control/status requires byte access",
                    ));
                }
                let value = value as u8;
                match offset {
                    CACR0 => {
                        state.control0 = value & CFME;
                        if state.control0 & CFME == 0 {
                            state.count = 0;
                            state.status = 0;
                        }
                    }
                    CACR1 => {
                        if state.control0 & CFME == 0 {
                            state.control1 = value;
                        }
                    }
                    CACR2 => {
                        if state.control0 & CFME == 0 {
                            state.control2 = value;
                        }
                    }
                    CAICR => {
                        state.interrupt = (state.interrupt & !(FERRFCL | MENDFCL | OVFFCL))
                            | (value & (FERRIE | MENDIE | OVFIE));
                        if value & FERRFCL != 0 {
                            state.status &= !(1 << 0);
                        }
                        if value & MENDFCL != 0 {
                            state.status &= !(1 << 1);
                        }
                        if value & OVFFCL != 0 {
                            state.status &= !(1 << 2);
                        }
                    }
                    CASTR => {}
                    _ => unreachable!(),
                }
            }
            CAULVR => {
                if width != AccessWidth::HalfWord {
                    return Err(DeviceError::new("RA CAC limits require half-word access"));
                }
                if state.control0 & CFME == 0 {
                    state.upper = value as u16;
                }
            }
            CALLVR => {
                if width != AccessWidth::HalfWord {
                    return Err(DeviceError::new("RA CAC limits require half-word access"));
                }
                if state.control0 & CFME == 0 {
                    state.lower = value as u16;
                }
            }
            CACNTBR => {
                return Err(DeviceError::new("RA CAC counter buffer is read-only"));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RA CAC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA CAC lock poisoned") = CacState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_measurement_sets_range_flags_and_clear_bits_work() {
        let (mut cac, handle) = RaCac::new("cac");
        cac.write(CAULVR, AccessWidth::HalfWord, 100, SimTime::ZERO)
            .unwrap();
        cac.write(CALLVR, AccessWidth::HalfWord, 50, SimTime::ZERO)
            .unwrap();
        cac.write(CAICR, AccessWidth::Byte, MENDIE.into(), SimTime::ZERO)
            .unwrap();
        cac.write(CACR0, AccessWidth::Byte, CFME.into(), SimTime::ZERO)
            .unwrap();
        handle.reference_edge(75);
        assert_eq!(handle.count(), 75);
        assert_eq!(handle.flags(), (false, true, false));
        handle.reference_edge(120);
        assert_eq!(handle.flags(), (true, true, false));
        cac.write(
            CAICR,
            AccessWidth::Byte,
            u64::from(FERRFCL | MENDFCL),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.flags(), (false, false, false));
    }

    #[test]
    fn disabling_measurement_clears_counter_and_status() {
        let (mut cac, handle) = RaCac::new("cac");
        cac.write(CAULVR, AccessWidth::HalfWord, 10, SimTime::ZERO)
            .unwrap();
        cac.write(CACR0, AccessWidth::Byte, CFME.into(), SimTime::ZERO)
            .unwrap();
        handle.reference_edge(5);
        cac.write(CACR0, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.count(), 0);
        assert_eq!(handle.flags(), (false, false, false));
    }
}
