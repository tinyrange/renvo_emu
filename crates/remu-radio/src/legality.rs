use crate::RadioProtocol;
use remu_core::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Silicon target whose firmware-derived radio contract is being enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioChip {
    /// ESP32-C6 revision-zero radio complex.
    Esp32C6,
    /// ESP32-S3 revision-zero radio complex.
    Esp32S3,
}

impl std::fmt::Display for RadioChip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Esp32C6 => "esp32c6",
            Self::Esp32S3 => "esp32s3",
        })
    }
}

/// Firmware-visible radio block participating in one legality rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioSubsystem {
    /// Wi-Fi MAC, DMA, baseband, and PHY-facing state.
    Wifi,
    /// Bluetooth LE controller, scheduler, exchange memory, and DMA.
    BluetoothLe,
    /// ESP32-C6 IEEE 802.15.4 MAC and DMA.
    Ieee802154,
    /// Shared RF coexistence arbiter.
    Coexistence,
}

impl RadioSubsystem {
    /// Returns the corresponding over-the-air protocol, if this is not the
    /// shared coexistence block.
    pub const fn protocol(self) -> Option<RadioProtocol> {
        match self {
            Self::Wifi => Some(RadioProtocol::Wifi),
            Self::BluetoothLe => Some(RadioProtocol::BluetoothLe),
            Self::Ieee802154 => Some(RadioProtocol::Ieee802154),
            Self::Coexistence => None,
        }
    }
}

impl std::fmt::Display for RadioSubsystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Wifi => "wifi",
            Self::BluetoothLe => "bluetooth-le",
            Self::Ieee802154 => "ieee802154",
            Self::Coexistence => "coexistence",
        })
    }
}

/// Stable firmware-derived invariant violated by a radio state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioLegalityRule {
    /// Observations must never move backward in deterministic machine time.
    MonotonicTime,
    /// A native operation requires its firmware-enabled clock/reset domain.
    DomainReady,
    /// Reset generations are monotonic for the lifetime of a machine.
    MonotonicResetGeneration,
    /// A firmware-owned DMA pointer must be nonzero and correctly aligned.
    DmaAddress,
    /// A firmware-owned DMA length must fit the recovered hardware field.
    DmaLength,
    /// A native interrupt requires firmware to have enabled its current domain generation.
    InterruptDomain,
    /// Mutually exclusive controller activities cannot overlap.
    OperationOverlap,
    /// A completion must correspond to an active firmware-started operation.
    CompletionWithoutOperation,
    /// Native scheduler state must use a firmware-observed state encoding.
    SchedulerState,
    /// Exchange-memory or linked-list mappings must resolve to guest memory.
    MemoryMapping,
    /// RF activity must agree with the active coexistence grant.
    CoexistenceOwnership,
}

impl RadioLegalityRule {
    /// Stable diagnostic code stored in qualification contracts and errors.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MonotonicTime => "monotonic-time",
            Self::DomainReady => "domain-ready",
            Self::MonotonicResetGeneration => "monotonic-reset-generation",
            Self::DmaAddress => "dma-address",
            Self::DmaLength => "dma-length",
            Self::InterruptDomain => "interrupt-domain",
            Self::OperationOverlap => "operation-overlap",
            Self::CompletionWithoutOperation => "completion-without-operation",
            Self::SchedulerState => "scheduler-state",
            Self::MemoryMapping => "memory-mapping",
            Self::CoexistenceOwnership => "coexistence-ownership",
        }
    }

    /// Complete stable rule-code inventory used by the source audit.
    pub const ALL: [Self; 11] = [
        Self::MonotonicTime,
        Self::DomainReady,
        Self::MonotonicResetGeneration,
        Self::DmaAddress,
        Self::DmaLength,
        Self::InterruptDomain,
        Self::OperationOverlap,
        Self::CompletionWithoutOperation,
        Self::SchedulerState,
        Self::MemoryMapping,
        Self::CoexistenceOwnership,
    ];
}

/// Direction of a firmware-owned radio DMA object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioDmaDirection {
    /// Guest memory consumed by the radio.
    Transmit,
    /// Guest memory produced by the radio.
    Receive,
}

/// Coarse native activity used to reject impossible controller combinations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioActivity {
    /// No RF or baseband operation is active.
    #[default]
    Idle,
    /// A transmit operation owns the native datapath.
    Transmit,
    /// A receive operation owns the native datapath.
    Receive,
    /// A single-shot clear-channel assessment is active.
    ClearChannelAssessment,
    /// An energy-detection measurement is active.
    EnergyDetection,
    /// A transmitted frame completed and hardware is awaiting its ACK.
    AwaitingAck,
}

/// Hard diagnostic produced for a radio state that genuine firmware never
/// configures and the native peripheral cannot legally represent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error(
    "illegal radio state [{chip}/{subsystem}/{code}] at {at}: {detail}",
    code = .rule.code()
)]
pub struct RadioLegalityError {
    /// Target whose contract rejected the state.
    pub chip: RadioChip,
    /// Native radio block involved in the violation.
    pub subsystem: RadioSubsystem,
    /// Stable violated rule.
    pub rule: RadioLegalityRule,
    /// Deterministic observation time.
    pub at: SimTime,
    /// Concrete register, DMA, interrupt, or transition evidence.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default)]
struct DomainState {
    observed: bool,
    ready: bool,
    ever_ready: bool,
    reset_generation: Option<u64>,
    activity: RadioActivity,
}

/// Runtime enforcer for the pinned, firmware-derived radio state contract.
///
/// The validator does not learn permissive states at runtime. Qualification
/// traces define the accepted rule inventory; every machine execution applies
/// those rules and terminates on the first contradiction.
#[derive(Clone, Debug)]
pub struct RadioLegalityValidator {
    chip: RadioChip,
    now: SimTime,
    domains: BTreeMap<RadioSubsystem, DomainState>,
}

impl RadioLegalityValidator {
    /// Creates a validator at reset time for one silicon contract.
    pub fn new(chip: RadioChip) -> Self {
        Self {
            chip,
            now: SimTime::ZERO,
            domains: BTreeMap::new(),
        }
    }

    /// Silicon contract enforced by this validator.
    pub const fn chip(&self) -> RadioChip {
        self.chip
    }

    /// Observes clock/reset eligibility and optionally a monotonic reset
    /// generation. A new reset generation retires in-flight activity before
    /// the new domain state is considered.
    pub fn observe_domain(
        &mut self,
        subsystem: RadioSubsystem,
        ready: bool,
        reset_generation: Option<u64>,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        let state = self.domains.entry(subsystem).or_default();
        if let (Some(previous), Some(current)) = (state.reset_generation, reset_generation) {
            if current < previous {
                return Err(Self::error_for(
                    self.chip,
                    subsystem,
                    RadioLegalityRule::MonotonicResetGeneration,
                    at,
                    format!("reset generation decreased from {previous} to {current}"),
                ));
            }
            if current != previous {
                state.activity = RadioActivity::Idle;
                state.ever_ready = false;
            }
        }
        if state.observed && state.ready && !ready && state.activity != RadioActivity::Idle {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::DomainReady,
                at,
                format!(
                    "domain became unavailable during {:?} without a reset transition",
                    state.activity
                ),
            ));
        }
        state.observed = true;
        state.ready = ready;
        state.ever_ready |= ready;
        state.reset_generation = reset_generation.or(state.reset_generation);
        Ok(())
    }

    /// Starts one mutually exclusive native controller activity.
    pub fn begin_activity(
        &mut self,
        subsystem: RadioSubsystem,
        activity: RadioActivity,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        let state = self.domains.entry(subsystem).or_default();
        if !state.observed || !state.ready {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::DomainReady,
                at,
                format!("firmware started {activity:?} while the domain was unavailable"),
            ));
        }
        if state.activity != RadioActivity::Idle {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::OperationOverlap,
                at,
                format!(
                    "firmware started {activity:?} while {:?} was active",
                    state.activity
                ),
            ));
        }
        state.activity = activity;
        Ok(())
    }

    /// Moves an active controller operation through a firmware-observed phase.
    pub fn transition_activity(
        &mut self,
        subsystem: RadioSubsystem,
        expected: RadioActivity,
        next: RadioActivity,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        let state = self.domains.entry(subsystem).or_default();
        if state.activity != expected {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::CompletionWithoutOperation,
                at,
                format!(
                    "transition expected {expected:?}, observed {:?}, requested {next:?}",
                    state.activity
                ),
            ));
        }
        state.activity = next;
        Ok(())
    }

    /// Retires native activity after an explicit reset/stop sequence.
    pub fn force_idle(
        &mut self,
        subsystem: RadioSubsystem,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        self.domains.entry(subsystem).or_default().activity = RadioActivity::Idle;
        Ok(())
    }

    /// Validates one firmware-owned DMA object before the emulator reads or
    /// writes it.
    pub fn validate_dma(
        &mut self,
        subsystem: RadioSubsystem,
        direction: RadioDmaDirection,
        address: u32,
        alignment: u32,
        length: usize,
        maximum_length: usize,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        if address == 0
            || alignment == 0
            || !alignment.is_power_of_two()
            || !address.is_multiple_of(alignment)
        {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::DmaAddress,
                at,
                format!(
                    "{direction:?} DMA address {address:#010x} is not nonzero/{alignment}-byte aligned"
                ),
            ));
        }
        if length == 0 || length > maximum_length {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::DmaLength,
                at,
                format!("{direction:?} DMA length {length} is outside 1..={maximum_length}"),
            ));
        }
        Ok(())
    }

    /// Rejects a native interrupt before genuine firmware has enabled the
    /// domain in its current reset generation. Firmware traces show that
    /// hardware may raise or retain status while its functional clock is
    /// subsequently gated, so current clock availability is deliberately not
    /// used as an interrupt legality condition.
    pub fn observe_interrupt(
        &mut self,
        subsystem: RadioSubsystem,
        pending: bool,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        let state = self.domains.entry(subsystem).or_default();
        if pending && !state.ever_ready {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::InterruptDomain,
                at,
                "native interrupt asserted before firmware enabled the domain generation"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Requires an over-the-air submission to agree with the protocol named
    /// by its active coexistence grant.
    ///
    /// A denied request is an ordinary modeled outcome and must not call this
    /// method because no RF submission follows it. This check guards the
    /// separate granted path against transmitting under another protocol's
    /// ownership.
    pub fn validate_coexistence_ownership(
        &mut self,
        subsystem: RadioSubsystem,
        submission: RadioProtocol,
        granted: RadioProtocol,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        self.require(
            subsystem,
            RadioLegalityRule::CoexistenceOwnership,
            submission == granted,
            at,
            format!(
                "{submission:?} RF submission attempted under {granted:?} coexistence ownership"
            ),
        )
    }

    /// Applies a recovered chip-specific invariant at a machine boundary.
    pub fn require(
        &mut self,
        subsystem: RadioSubsystem,
        rule: RadioLegalityRule,
        condition: bool,
        at: SimTime,
        detail: impl Into<String>,
    ) -> Result<(), RadioLegalityError> {
        self.observe_time(subsystem, at)?;
        if condition {
            Ok(())
        } else {
            Err(Self::error_for(
                self.chip,
                subsystem,
                rule,
                at,
                detail.into(),
            ))
        }
    }

    fn observe_time(
        &mut self,
        subsystem: RadioSubsystem,
        at: SimTime,
    ) -> Result<(), RadioLegalityError> {
        if at < self.now {
            return Err(Self::error_for(
                self.chip,
                subsystem,
                RadioLegalityRule::MonotonicTime,
                at,
                format!("validator time moved backward from {} to {at}", self.now),
            ));
        }
        self.now = at;
        Ok(())
    }

    fn error_for(
        chip: RadioChip,
        subsystem: RadioSubsystem,
        rule: RadioLegalityRule,
        at: SimTime,
        detail: String,
    ) -> RadioLegalityError {
        RadioLegalityError {
            chip,
            subsystem,
            rule,
            at,
            detail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_activity_before_firmware_enables_the_domain() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32C6);
        validator
            .observe_domain(RadioSubsystem::Ieee802154, false, Some(1), SimTime::ZERO)
            .unwrap();
        let error = validator
            .begin_activity(
                RadioSubsystem::Ieee802154,
                RadioActivity::Transmit,
                SimTime::from_ticks(1),
            )
            .unwrap_err();
        assert_eq!(error.rule, RadioLegalityRule::DomainReady);
        assert!(error.to_string().contains("illegal radio state"));
    }

    #[test]
    fn reset_generation_retires_activity_but_clock_gating_does_not() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32C6);
        validator
            .observe_domain(RadioSubsystem::BluetoothLe, true, Some(1), SimTime::ZERO)
            .unwrap();
        validator
            .begin_activity(
                RadioSubsystem::BluetoothLe,
                RadioActivity::Receive,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            validator
                .observe_domain(
                    RadioSubsystem::BluetoothLe,
                    false,
                    Some(1),
                    SimTime::from_ticks(2),
                )
                .unwrap_err()
                .rule,
            RadioLegalityRule::DomainReady
        );
        validator
            .observe_domain(
                RadioSubsystem::BluetoothLe,
                false,
                Some(2),
                SimTime::from_ticks(3),
            )
            .unwrap();
    }

    #[test]
    fn rejects_overlapping_operations_and_unmatched_completions() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32S3);
        validator
            .observe_domain(RadioSubsystem::BluetoothLe, true, None, SimTime::ZERO)
            .unwrap();
        validator
            .begin_activity(
                RadioSubsystem::BluetoothLe,
                RadioActivity::Transmit,
                SimTime::from_ticks(1),
            )
            .unwrap();
        assert_eq!(
            validator
                .begin_activity(
                    RadioSubsystem::BluetoothLe,
                    RadioActivity::Receive,
                    SimTime::from_ticks(1),
                )
                .unwrap_err()
                .rule,
            RadioLegalityRule::OperationOverlap
        );
        assert_eq!(
            validator
                .transition_activity(
                    RadioSubsystem::BluetoothLe,
                    RadioActivity::Receive,
                    RadioActivity::Idle,
                    SimTime::from_ticks(2),
                )
                .unwrap_err()
                .rule,
            RadioLegalityRule::CompletionWithoutOperation
        );
    }

    #[test]
    fn validates_firmware_owned_dma_shape() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32S3);
        validator
            .validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                0x3fc8_1000,
                4,
                128,
                4095,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            validator
                .validate_dma(
                    RadioSubsystem::Wifi,
                    RadioDmaDirection::Transmit,
                    0x3fc8_1002,
                    4,
                    128,
                    4095,
                    SimTime::ZERO,
                )
                .unwrap_err()
                .rule,
            RadioLegalityRule::DmaAddress
        );
    }

    #[test]
    fn permits_interrupts_after_clock_gating_but_rejects_pre_enable_assertion() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32S3);
        validator
            .observe_domain(RadioSubsystem::Wifi, false, None, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            validator
                .observe_interrupt(RadioSubsystem::Wifi, true, SimTime::from_ticks(1))
                .unwrap_err()
                .rule,
            RadioLegalityRule::InterruptDomain
        );
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32S3);
        validator
            .observe_domain(RadioSubsystem::Wifi, true, None, SimTime::ZERO)
            .unwrap();
        validator
            .observe_interrupt(RadioSubsystem::Wifi, true, SimTime::from_ticks(1))
            .unwrap();
        validator
            .observe_domain(RadioSubsystem::Wifi, false, None, SimTime::from_ticks(2))
            .unwrap();
        validator
            .observe_interrupt(RadioSubsystem::Wifi, true, SimTime::from_ticks(2))
            .unwrap();
        validator
            .observe_interrupt(RadioSubsystem::Wifi, false, SimTime::from_ticks(3))
            .unwrap();
        validator
            .observe_interrupt(RadioSubsystem::Wifi, true, SimTime::from_ticks(4))
            .unwrap();
    }

    #[test]
    fn rejects_rf_submission_under_another_protocols_coexistence_grant() {
        let mut validator = RadioLegalityValidator::new(RadioChip::Esp32C6);
        validator
            .validate_coexistence_ownership(
                RadioSubsystem::Wifi,
                RadioProtocol::Wifi,
                RadioProtocol::Wifi,
                SimTime::ZERO,
            )
            .unwrap();
        let error = validator
            .validate_coexistence_ownership(
                RadioSubsystem::Wifi,
                RadioProtocol::Wifi,
                RadioProtocol::BluetoothLe,
                SimTime::from_ticks(1),
            )
            .unwrap_err();
        assert_eq!(error.rule, RadioLegalityRule::CoexistenceOwnership);
        assert!(error.to_string().starts_with("illegal radio state ["));
    }
}
