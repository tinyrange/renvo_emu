//! Four-state digital signals, resolved nets, and timestamped changes.

use remu_core::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

/// One bit in the digital four-state domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Logic {
    /// Driven digital low.
    Zero,
    /// Driven digital high.
    One,
    /// High impedance / no drive.
    #[default]
    Z,
    /// Unknown, conflicting, or invalid.
    X,
}

impl Logic {
    /// VCD scalar representation.
    pub const fn vcd(self) -> char {
        match self {
            Self::Zero => '0',
            Self::One => '1',
            Self::Z => 'z',
            Self::X => 'x',
        }
    }
}

impl fmt::Display for Logic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.vcd().to_string())
    }
}

/// Fixed-width, least-significant-bit-first four-state value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignalValue {
    bits: Vec<Logic>,
}

impl SignalValue {
    /// Constructs a value. At least one bit is required.
    pub fn new(bits: Vec<Logic>) -> Result<Self, SignalError> {
        if bits.is_empty() {
            return Err(SignalError::ZeroWidth);
        }
        Ok(Self { bits })
    }

    /// Constructs a uniformly initialized value.
    pub fn repeat(bit: Logic, width: u16) -> Result<Self, SignalError> {
        if width == 0 {
            return Err(SignalError::ZeroWidth);
        }
        Ok(Self {
            bits: vec![bit; usize::from(width)],
        })
    }

    /// Constructs a known value from the low `width` bits.
    pub fn from_u64(value: u64, width: u16) -> Result<Self, SignalError> {
        if width == 0 {
            return Err(SignalError::ZeroWidth);
        }
        let bits = (0..width)
            .map(|index| {
                if index >= 64 || value & (1_u64 << u32::from(index)) == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                }
            })
            .collect();
        Ok(Self { bits })
    }

    /// Number of bits.
    pub fn width(&self) -> u16 {
        u16::try_from(self.bits.len()).expect("signal width is constructed from u16")
    }

    /// Least-significant-bit-first representation.
    pub fn bits(&self) -> &[Logic] {
        &self.bits
    }

    /// Returns one bit by index.
    pub fn bit(&self, index: u16) -> Option<Logic> {
        self.bits.get(usize::from(index)).copied()
    }

    /// VCD binary representation, most-significant bit first.
    pub fn to_vcd_binary(&self) -> String {
        self.bits.iter().rev().map(|bit| bit.vcd()).collect()
    }
}

/// Stable registry identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SignalId(u32);

impl SignalId {
    /// Integer ID used in artifacts.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Stable driver identity within a net.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DriverId(pub u32);

/// Signal metadata declared before tracing starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDescriptor {
    /// Stable signal ID.
    pub id: SignalId,
    /// Dot-separated hierarchy and leaf name.
    pub path: String,
    /// Bit width.
    pub width: u16,
    /// Optional description.
    pub description: Option<String>,
}

/// One timestamped value transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalChange {
    /// Simulation timestamp.
    pub at: SimTime,
    /// Changed signal.
    pub signal: SignalId,
    /// Previous value.
    pub previous: SignalValue,
    /// New value.
    pub value: SignalValue,
}

/// Signal declaration or update error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SignalError {
    /// Zero-width signals are not representable.
    #[error("signal width must be non-zero")]
    ZeroWidth,
    /// Path is empty or contains an empty hierarchy component.
    #[error("invalid signal path {0:?}")]
    InvalidPath(String),
    /// Signal path already exists.
    #[error("signal path {0:?} is already declared")]
    DuplicatePath(String),
    /// Signal ID is not declared.
    #[error("unknown signal ID {0}")]
    UnknownSignal(u32),
    /// Signal path is not declared.
    #[error("unknown signal path {0:?}")]
    UnknownPath(String),
    /// New value width differs from the declaration.
    #[error("signal width mismatch: expected {expected}, received {actual}")]
    WidthMismatch {
        /// Declared width.
        expected: u16,
        /// New value width.
        actual: u16,
    },
    /// Signal ID space was exhausted.
    #[error("signal ID space exhausted")]
    IdExhausted,
}

/// Registry of declared signals and current values.
#[derive(Debug, Default)]
pub struct SignalRegistry {
    next_id: u32,
    descriptors: BTreeMap<SignalId, SignalDescriptor>,
    paths: BTreeMap<String, SignalId>,
    values: BTreeMap<SignalId, SignalValue>,
}

impl SignalRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a signal and its initial value.
    pub fn declare(
        &mut self,
        path: impl Into<String>,
        initial: SignalValue,
        description: Option<String>,
    ) -> Result<SignalId, SignalError> {
        let path = path.into();
        if path.is_empty() || path.split('.').any(str::is_empty) {
            return Err(SignalError::InvalidPath(path));
        }
        if self.paths.contains_key(&path) {
            return Err(SignalError::DuplicatePath(path));
        }
        let id = SignalId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(SignalError::IdExhausted)?;
        let descriptor = SignalDescriptor {
            id,
            path: path.clone(),
            width: initial.width(),
            description,
        };
        self.descriptors.insert(id, descriptor);
        self.paths.insert(path, id);
        self.values.insert(id, initial);
        Ok(id)
    }

    /// Updates a signal, returning `None` when its value did not change.
    pub fn set(
        &mut self,
        signal: SignalId,
        value: SignalValue,
        at: SimTime,
    ) -> Result<Option<SignalChange>, SignalError> {
        let descriptor = self
            .descriptors
            .get(&signal)
            .ok_or(SignalError::UnknownSignal(signal.get()))?;
        if value.width() != descriptor.width {
            return Err(SignalError::WidthMismatch {
                expected: descriptor.width,
                actual: value.width(),
            });
        }
        let current = self
            .values
            .get_mut(&signal)
            .expect("descriptor and current signal value are inserted together");
        if *current == value {
            return Ok(None);
        }
        let previous = core::mem::replace(current, value.clone());
        Ok(Some(SignalChange {
            at,
            signal,
            previous,
            value,
        }))
    }

    /// Returns descriptors in stable signal-ID order.
    pub fn descriptors(&self) -> impl Iterator<Item = &SignalDescriptor> {
        self.descriptors.values()
    }

    /// Returns a descriptor.
    pub fn descriptor(&self, signal: SignalId) -> Option<&SignalDescriptor> {
        self.descriptors.get(&signal)
    }

    /// Returns the current value.
    pub fn value(&self, signal: SignalId) -> Option<&SignalValue> {
        self.values.get(&signal)
    }

    /// Resolves a declared path.
    pub fn find(&self, path: &str) -> Option<SignalId> {
        self.paths.get(path).copied()
    }
}

/// Result of changing one resolved net driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetUpdate {
    /// Previous resolved state.
    pub previous: Logic,
    /// New resolved state.
    pub value: Logic,
    /// True when two known active drivers conflict.
    pub contention: bool,
}

/// One-bit net with deterministic multi-driver resolution.
#[derive(Clone, Debug, Default)]
pub struct DigitalNet {
    drivers: BTreeMap<DriverId, Logic>,
    resolved: Logic,
}

impl DigitalNet {
    /// Creates an undriven high-impedance net.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current resolved state.
    pub const fn resolved(&self) -> Logic {
        self.resolved
    }

    /// Drives or updates one source.
    pub fn drive(&mut self, driver: DriverId, value: Logic) -> NetUpdate {
        let previous = self.resolved;
        self.drivers.insert(driver, value);
        let (resolved, contention) = resolve(self.drivers.values().copied());
        self.resolved = resolved;
        NetUpdate {
            previous,
            value: resolved,
            contention,
        }
    }

    /// Removes one source, equivalent to disconnecting it.
    pub fn disconnect(&mut self, driver: DriverId) -> NetUpdate {
        let previous = self.resolved;
        self.drivers.remove(&driver);
        let (resolved, contention) = resolve(self.drivers.values().copied());
        self.resolved = resolved;
        NetUpdate {
            previous,
            value: resolved,
            contention,
        }
    }
}

fn resolve(drivers: impl Iterator<Item = Logic>) -> (Logic, bool) {
    let mut known: Option<Logic> = None;
    let mut unknown = false;
    let mut contention = false;
    for driver in drivers {
        match driver {
            Logic::Z => {}
            Logic::X => unknown = true,
            Logic::Zero | Logic::One => {
                if let Some(existing) = known {
                    if existing != driver {
                        contention = true;
                    }
                } else {
                    known = Some(driver);
                }
            }
        }
    }
    if contention || unknown {
        (Logic::X, contention)
    } else {
        (known.unwrap_or(Logic::Z), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_suppresses_identical_changes() {
        let mut registry = SignalRegistry::new();
        let low = SignalValue::from_u64(0, 1).unwrap();
        let id = registry.declare("board.led", low.clone(), None).unwrap();
        assert_eq!(registry.set(id, low, SimTime::ZERO).unwrap(), None);
        let change = registry
            .set(
                id,
                SignalValue::from_u64(1, 1).unwrap(),
                SimTime::from_ticks(3),
            )
            .unwrap()
            .unwrap();
        assert_eq!(change.at, SimTime::from_ticks(3));
        assert_eq!(change.value.bit(0), Some(Logic::One));
    }

    #[test]
    fn net_reports_contention() {
        let mut net = DigitalNet::new();
        assert_eq!(net.drive(DriverId(1), Logic::Zero).value, Logic::Zero);
        let update = net.drive(DriverId(2), Logic::One);
        assert_eq!(update.value, Logic::X);
        assert!(update.contention);
        assert_eq!(net.disconnect(DriverId(2)).value, Logic::Zero);
    }

    #[test]
    fn vcd_binary_is_most_significant_bit_first() {
        let value = SignalValue::from_u64(0b1001, 4).unwrap();
        assert_eq!(value.to_vcd_binary(), "1001");
    }
}
