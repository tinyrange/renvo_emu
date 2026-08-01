use core::fmt;
use core::ops::{Add, AddAssign, Sub};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Monotonic simulation timestamp in machine-defined ticks.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimTime(u64);

/// Duration measured in machine-defined simulation ticks.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimDuration(u64);

/// Error returned by checked simulation-time arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TimeError {
    /// The requested timestamp cannot be represented.
    #[error("simulation time overflow")]
    Overflow,
}

impl SimTime {
    /// Beginning of the simulation timeline.
    pub const ZERO: Self = Self(0);

    /// Constructs a timestamp from raw machine ticks.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    /// Returns the raw machine tick count.
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Adds a duration, reporting overflow.
    pub fn checked_add(self, duration: SimDuration) -> Result<Self, TimeError> {
        self.0
            .checked_add(duration.0)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }

    /// Returns the duration since `earlier`, or `None` when it is in the future.
    pub fn checked_duration_since(self, earlier: Self) -> Option<SimDuration> {
        self.0.checked_sub(earlier.0).map(SimDuration)
    }
}

impl SimDuration {
    /// A duration containing no ticks.
    pub const ZERO: Self = Self(0);

    /// One simulation tick.
    pub const TICK: Self = Self(1);

    /// Constructs a duration from raw machine ticks.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    /// Returns the raw machine tick count.
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Multiplies a duration, reporting overflow.
    pub fn checked_mul(self, factor: u64) -> Result<Self, TimeError> {
        self.0
            .checked_mul(factor)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }
}

impl Add<SimDuration> for SimTime {
    type Output = Self;

    fn add(self, rhs: SimDuration) -> Self::Output {
        self.checked_add(rhs)
            .expect("simulation time overflow; use SimTime::checked_add")
    }
}

impl AddAssign<SimDuration> for SimTime {
    fn add_assign(&mut self, rhs: SimDuration) {
        *self = *self + rhs;
    }
}

impl Sub for SimTime {
    type Output = SimDuration;

    fn sub(self, rhs: Self) -> Self::Output {
        self.checked_duration_since(rhs)
            .expect("cannot subtract a later simulation timestamp")
    }
}

impl fmt::Debug for SimTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}t", self.0)
    }
}

impl fmt::Display for SimTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl fmt::Debug for SimDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}t", self.0)
    }
}

impl fmt::Display for SimDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_time_math_detects_overflow() {
        let near_end = SimTime::from_ticks(u64::MAX);
        assert_eq!(
            near_end.checked_add(SimDuration::TICK),
            Err(TimeError::Overflow)
        );
    }

    #[test]
    fn duration_since_rejects_future_time() {
        assert_eq!(
            SimTime::from_ticks(2).checked_duration_since(SimTime::from_ticks(3)),
            None
        );
    }
}
