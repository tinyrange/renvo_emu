use crate::RadioProtocol;
use remu_core::{SimDuration, SimTime, TimeError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable identifier for one RF coexistence grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoexistenceGrantId(pub u64);

/// Request by one protocol to own a chip's shared RF path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoexistenceRequest {
    /// Requesting protocol.
    pub protocol: RadioProtocol,
    /// Earliest requested ownership time.
    pub start: SimTime,
    /// Required contiguous ownership duration.
    pub duration: SimDuration,
    /// Higher numeric values win preemption decisions.
    pub priority: u8,
    /// Whether a later higher-priority request may truncate this grant.
    pub preemptible: bool,
}

/// Immediate deterministic arbitration result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "kebab-case")]
pub enum CoexistenceDecision {
    /// RF ownership was granted for the requested interval.
    Granted {
        /// Stable grant identifier.
        id: CoexistenceGrantId,
        /// Protocol that owns the granted interval.
        protocol: RadioProtocol,
        /// Prior grant truncated by this higher-priority request, if any.
        preempted: Option<CoexistenceGrantId>,
        /// Inclusive ownership start.
        start: SimTime,
        /// Exclusive ownership end.
        end: SimTime,
    },
    /// The current grant retained ownership.
    Denied {
        /// End of the conflicting grant.
        occupied_until: SimTime,
        /// Protocol retaining ownership.
        owner: RadioProtocol,
    },
}

/// Append-only arbitration evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum CoexistenceEvent {
    /// A protocol obtained RF ownership.
    Granted {
        /// Grant identifier.
        id: CoexistenceGrantId,
        /// Protocol receiving the grant.
        protocol: RadioProtocol,
        /// Inclusive ownership start.
        start: SimTime,
        /// Original or truncated ownership end.
        end: SimTime,
        /// Request priority.
        priority: u8,
    },
    /// A grant was truncated by higher-priority work.
    Preempted {
        /// Truncated grant identifier.
        id: CoexistenceGrantId,
        /// Protocol that lost ownership.
        protocol: RadioProtocol,
        /// Time at which ownership ended.
        at: SimTime,
        /// Protocol taking ownership.
        by: RadioProtocol,
    },
    /// A request lost arbitration.
    Denied {
        /// Denied protocol.
        protocol: RadioProtocol,
        /// Requested start.
        at: SimTime,
        /// Current owner.
        owner: RadioProtocol,
        /// End of the conflicting grant.
        occupied_until: SimTime,
    },
    /// Active ownership was released early.
    Released {
        /// Released grant identifier.
        id: CoexistenceGrantId,
        /// Releasing protocol.
        protocol: RadioProtocol,
        /// Release time.
        at: SimTime,
    },
    /// Firmware reset canceled any active ownership.
    Reset {
        /// Reset observation time.
        at: SimTime,
    },
    /// Firmware clock/power gating canceled active ownership.
    PowerDown {
        /// Canceled grant identifier.
        id: CoexistenceGrantId,
        /// Protocol that lost RF power.
        protocol: RadioProtocol,
        /// Power-down observation time.
        at: SimTime,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveGrant {
    id: CoexistenceGrantId,
    protocol: RadioProtocol,
    end: SimTime,
    priority: u8,
    preemptible: bool,
}

/// Coexistence arbitration error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CoexistenceError {
    /// A request or release attempted to move backward in time.
    #[error("coexistence time moved backward from {now} to {requested}")]
    TimeReversal {
        /// Current arbiter time.
        now: SimTime,
        /// Requested time.
        requested: SimTime,
    },
    /// A zero-duration request is invalid.
    #[error("coexistence request duration must be positive")]
    ZeroDuration,
    /// Grant end time cannot be represented.
    #[error(transparent)]
    Time(#[from] TimeError),
    /// Stable grant identifiers have been exhausted.
    #[error("coexistence grant identifier space exhausted")]
    IdentifierExhausted,
}

/// Deterministic single-RF-path arbiter shared by radio frontends on one chip.
#[derive(Clone, Debug, Default)]
pub struct CoexistenceArbiter {
    now: SimTime,
    next_id: u64,
    active: Option<ActiveGrant>,
    events: Vec<CoexistenceEvent>,
}

impl CoexistenceArbiter {
    /// Creates an idle arbiter at simulation time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current monotonic arbitration time.
    pub const fn now(&self) -> SimTime {
        self.now
    }

    /// Requests shared RF ownership.
    ///
    /// Equal-priority ties retain the existing grant, making insertion order
    /// an explicit and replayable tie-breaker.
    pub fn request(
        &mut self,
        request: CoexistenceRequest,
    ) -> Result<CoexistenceDecision, CoexistenceError> {
        self.advance_to(request.start)?;
        if request.duration == SimDuration::ZERO {
            return Err(CoexistenceError::ZeroDuration);
        }
        let end = request.start.checked_add(request.duration)?;
        let mut preempted = None;
        if let Some(active) = self.active {
            if request.priority > active.priority && active.preemptible {
                preempted = Some(active.id);
                self.events.push(CoexistenceEvent::Preempted {
                    id: active.id,
                    protocol: active.protocol,
                    at: request.start,
                    by: request.protocol,
                });
            } else {
                self.events.push(CoexistenceEvent::Denied {
                    protocol: request.protocol,
                    at: request.start,
                    owner: active.protocol,
                    occupied_until: active.end,
                });
                return Ok(CoexistenceDecision::Denied {
                    occupied_until: active.end,
                    owner: active.protocol,
                });
            }
        }
        let id = CoexistenceGrantId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(CoexistenceError::IdentifierExhausted)?;
        self.active = Some(ActiveGrant {
            id,
            protocol: request.protocol,
            end,
            priority: request.priority,
            preemptible: request.preemptible,
        });
        self.events.push(CoexistenceEvent::Granted {
            id,
            protocol: request.protocol,
            start: request.start,
            end,
            priority: request.priority,
        });
        Ok(CoexistenceDecision::Granted {
            id,
            protocol: request.protocol,
            preempted,
            start: request.start,
            end,
        })
    }

    /// Advances time and expires a completed grant.
    pub fn advance_to(&mut self, requested: SimTime) -> Result<(), CoexistenceError> {
        if requested < self.now {
            return Err(CoexistenceError::TimeReversal {
                now: self.now,
                requested,
            });
        }
        self.now = requested;
        if self.active.is_some_and(|active| active.end <= requested) {
            self.active = None;
        }
        Ok(())
    }

    /// Releases a matching active grant. Stale releases are harmless.
    pub fn release(
        &mut self,
        id: CoexistenceGrantId,
        at: SimTime,
    ) -> Result<bool, CoexistenceError> {
        self.advance_to(at)?;
        let Some(active) = self.active else {
            return Ok(false);
        };
        if active.id != id {
            return Ok(false);
        }
        self.events.push(CoexistenceEvent::Released {
            id,
            protocol: active.protocol,
            at,
        });
        self.active = None;
        Ok(true)
    }

    /// Current RF owner and grant end.
    pub fn owner(&self) -> Option<(RadioProtocol, SimTime)> {
        self.active.map(|active| (active.protocol, active.end))
    }

    /// Active grant identity, protocol, and exclusive end timestamp.
    pub fn active_grant(&self) -> Option<(CoexistenceGrantId, RadioProtocol, SimTime)> {
        self.active
            .map(|active| (active.id, active.protocol, active.end))
    }

    /// Append-only arbitration evidence.
    pub fn events(&self) -> &[CoexistenceEvent] {
        &self.events
    }

    /// Clears ownership while preserving deterministic ID order and appending
    /// reset evidence.
    pub fn reset(&mut self, at: SimTime) -> Result<(), CoexistenceError> {
        self.advance_to(at)?;
        self.active = None;
        self.events.push(CoexistenceEvent::Reset { at });
        Ok(())
    }

    /// Cancels active ownership when firmware removes the required radio
    /// clock/power domain. Idle power transitions do not add noise to the
    /// arbitration artifact.
    pub fn power_down(&mut self, at: SimTime) -> Result<bool, CoexistenceError> {
        self.advance_to(at)?;
        let Some(active) = self.active.take() else {
            return Ok(false);
        };
        self.events.push(CoexistenceEvent::PowerDown {
            id: active.id,
            protocol: active.protocol,
            at,
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        protocol: RadioProtocol,
        start: u64,
        duration: u64,
        priority: u8,
        preemptible: bool,
    ) -> CoexistenceRequest {
        CoexistenceRequest {
            protocol,
            start: SimTime::from_ticks(start),
            duration: SimDuration::from_ticks(duration),
            priority,
            preemptible,
        }
    }

    #[test]
    fn higher_priority_preempts_only_preemptible_grant() {
        let mut arbiter = CoexistenceArbiter::new();
        let CoexistenceDecision::Granted {
            id: wifi_grant,
            protocol: RadioProtocol::Wifi,
            preempted: None,
            ..
        } = arbiter
            .request(request(RadioProtocol::Wifi, 10, 20, 1, true))
            .unwrap()
        else {
            panic!("initial Wi-Fi request was not granted");
        };
        assert!(matches!(
            arbiter
                .request(request(RadioProtocol::BluetoothLe, 15, 5, 2, false))
                .unwrap(),
            CoexistenceDecision::Granted {
                protocol: RadioProtocol::BluetoothLe,
                preempted: Some(id),
                ..
            } if id == wifi_grant
        ));
        assert!(arbiter.events().iter().any(|event| matches!(
            event,
            CoexistenceEvent::Preempted {
                protocol: RadioProtocol::Wifi,
                by: RadioProtocol::BluetoothLe,
                ..
            }
        )));
    }

    #[test]
    fn equal_priority_is_stable_and_existing_grant_wins() {
        let mut arbiter = CoexistenceArbiter::new();
        arbiter
            .request(request(RadioProtocol::Wifi, 0, 10, 3, true))
            .unwrap();
        assert_eq!(
            arbiter
                .request(request(RadioProtocol::Ieee802154, 0, 2, 3, true))
                .unwrap(),
            CoexistenceDecision::Denied {
                occupied_until: SimTime::from_ticks(10),
                owner: RadioProtocol::Wifi,
            }
        );
    }

    #[test]
    fn completed_grant_expires_and_release_is_id_checked() {
        let mut arbiter = CoexistenceArbiter::new();
        let decision = arbiter
            .request(request(RadioProtocol::Wifi, 4, 3, 1, false))
            .unwrap();
        let CoexistenceDecision::Granted { id, .. } = decision else {
            panic!("request was denied");
        };
        assert!(
            !arbiter
                .release(CoexistenceGrantId(id.0 + 1), SimTime::from_ticks(5))
                .unwrap()
        );
        arbiter.advance_to(SimTime::from_ticks(7)).unwrap();
        assert_eq!(arbiter.owner(), None);
    }

    #[test]
    fn reset_cancels_ownership_and_preserves_append_only_evidence() {
        let mut arbiter = CoexistenceArbiter::new();
        arbiter
            .request(request(RadioProtocol::Wifi, 4, 20, 1, true))
            .unwrap();
        arbiter.reset(SimTime::from_ticks(8)).unwrap();
        assert_eq!(arbiter.owner(), None);
        assert!(matches!(
            arbiter.events(),
            [CoexistenceEvent::Granted { .. }, CoexistenceEvent::Reset { at }]
                if *at == SimTime::from_ticks(8)
        ));
    }

    #[test]
    fn power_down_cancels_only_active_ownership() {
        let mut arbiter = CoexistenceArbiter::new();
        assert!(!arbiter.power_down(SimTime::ZERO).unwrap());
        let CoexistenceDecision::Granted { id, .. } = arbiter
            .request(request(RadioProtocol::Wifi, 4, 20, 1, true))
            .unwrap()
        else {
            panic!("Wi-Fi request was denied");
        };
        assert!(arbiter.power_down(SimTime::from_ticks(8)).unwrap());
        assert_eq!(arbiter.owner(), None);
        assert!(matches!(
            arbiter.events().last(),
            Some(CoexistenceEvent::PowerDown {
                id: event_id,
                protocol: RadioProtocol::Wifi,
                at,
            }) if *event_id == id && *at == SimTime::from_ticks(8)
        ));
    }
}
