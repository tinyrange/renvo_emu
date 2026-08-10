use crate::{
    DeliveryOutcome, MediumEvent, MediumProfile, NodeId, RadioProtocol, Receiver, TransmissionId,
    TxRequest,
};
use remu_core::SimTime;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug)]
struct PendingTransmission {
    id: TransmissionId,
    request: TxRequest,
    resolved: bool,
}

/// Error returned when a medium operation violates its deterministic contract.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MediumError {
    /// Packet loss must be between zero and one million parts per million.
    #[error("loss probability {0} ppm exceeds 1,000,000")]
    InvalidLoss(u32),
    /// Receiver bandwidth must be non-zero.
    #[error("receiver bandwidth must be non-zero")]
    InvalidReceiverBandwidth,
    /// A node can register at most one receiver for each protocol.
    #[error("receiver for node {node:?} and protocol {protocol:?} already exists")]
    DuplicateReceiver {
        /// Conflicting node.
        node: NodeId,
        /// Conflicting protocol.
        protocol: RadioProtocol,
    },
    /// Transmission bandwidth must be non-zero.
    #[error("transmission bandwidth must be non-zero")]
    InvalidTransmissionBandwidth,
    /// Transmission must contain a non-empty PHY name.
    #[error("transmission PHY name must not be empty")]
    MissingPhy,
    /// Transmission interval must have positive length.
    #[error("transmission must end after it starts")]
    InvalidInterval,
    /// New work may not be inserted into elapsed simulation time.
    #[error("transmission starts at {start}, before current medium time {now}")]
    StartsInPast {
        /// Requested start.
        start: SimTime,
        /// Current medium timestamp.
        now: SimTime,
    },
    /// Simulation time is monotonic.
    #[error("cannot move medium backward from {now} to {requested}")]
    TimeReversal {
        /// Current medium timestamp.
        now: SimTime,
        /// Requested timestamp.
        requested: SimTime,
    },
    /// Stable transmission identifiers have been exhausted.
    #[error("transmission identifier space exhausted")]
    IdentifierExhausted,
}

/// Deterministic shared RF medium with no implicit host networking.
#[derive(Clone, Debug)]
pub struct RadioMedium {
    profile: MediumProfile,
    now: SimTime,
    next_id: u64,
    receivers: BTreeMap<(NodeId, RadioProtocol), Receiver>,
    path_loss_db: BTreeMap<(NodeId, NodeId), i16>,
    transmissions: Vec<PendingTransmission>,
    events: Vec<MediumEvent>,
}

impl RadioMedium {
    /// Creates an empty medium at simulation time zero.
    pub fn new(profile: MediumProfile) -> Result<Self, MediumError> {
        if profile.loss_ppm > 1_000_000 {
            return Err(MediumError::InvalidLoss(profile.loss_ppm));
        }
        Ok(Self {
            profile,
            now: SimTime::ZERO,
            next_id: 0,
            receivers: BTreeMap::new(),
            path_loss_db: BTreeMap::new(),
            transmissions: Vec::new(),
            events: Vec::new(),
        })
    }

    /// Current monotonic medium timestamp.
    pub const fn now(&self) -> SimTime {
        self.now
    }

    /// Immutable deterministic profile.
    pub const fn profile(&self) -> &MediumProfile {
        &self.profile
    }

    /// Registers a receiver. Registration order never affects delivery order.
    pub fn register_receiver(&mut self, receiver: Receiver) -> Result<(), MediumError> {
        if receiver.spectrum.bandwidth_khz == 0 {
            return Err(MediumError::InvalidReceiverBandwidth);
        }
        let key = (receiver.node, receiver.protocol);
        if self.receivers.insert(key, receiver).is_some() {
            return Err(MediumError::DuplicateReceiver {
                node: key.0,
                protocol: key.1,
            });
        }
        Ok(())
    }

    /// Replaces an existing receiver tuning/configuration, or inserts it.
    pub fn tune_receiver(&mut self, receiver: Receiver) -> Result<(), MediumError> {
        if receiver.spectrum.bandwidth_khz == 0 {
            return Err(MediumError::InvalidReceiverBandwidth);
        }
        self.receivers
            .insert((receiver.node, receiver.protocol), receiver);
        Ok(())
    }

    /// Removes one protocol receiver from a node.
    pub fn remove_receiver(&mut self, node: NodeId, protocol: RadioProtocol) -> Option<Receiver> {
        self.receivers.remove(&(node, protocol))
    }

    /// Sets symmetric path loss for a node pair in integer dB.
    pub fn set_path_loss(&mut self, first: NodeId, second: NodeId, loss_db: i16) {
        self.path_loss_db
            .insert(ordered_node_pair(first, second), loss_db);
    }

    /// Computes the power observed by `receiver` for a transmission from
    /// `source`, using the same link-loss configuration as packet delivery.
    pub fn received_power_dbm(
        &self,
        source: NodeId,
        receiver: NodeId,
        transmit_power_dbm: i16,
    ) -> i16 {
        let path_loss = self
            .path_loss_db
            .get(&ordered_node_pair(source, receiver))
            .copied()
            .unwrap_or(self.profile.default_path_loss_db);
        transmit_power_dbm.saturating_sub(path_loss)
    }

    /// Returns the strongest currently active signal overlapping `spectrum`.
    ///
    /// The empty-medium floor is -128 dBm. This read has no side effects and
    /// is suitable for protocol CCA/energy-detect logic.
    pub fn energy_dbm_at(&self, receiver: NodeId, spectrum: crate::Spectrum) -> i16 {
        self.transmissions
            .iter()
            .filter(|transmission| {
                transmission.request.source != receiver
                    && transmission.request.start <= self.now
                    && transmission.request.end > self.now
                    && transmission.request.frame.spectrum.overlaps(spectrum)
            })
            .map(|transmission| self.received_power(transmission, receiver))
            .max()
            .unwrap_or(-128)
    }

    /// Returns whether a compatible active carrier overlaps `spectrum`.
    ///
    /// Unlike energy detection, this ignores transmissions from other radio
    /// protocols. The minimum power keeps a vanishingly weak packet from
    /// becoming a digital carrier merely because it exists in the event set.
    pub fn carrier_present_at(
        &self,
        receiver: NodeId,
        protocol: RadioProtocol,
        spectrum: crate::Spectrum,
        minimum_power_dbm: i16,
    ) -> bool {
        self.transmissions.iter().any(|transmission| {
            transmission.request.source != receiver
                && transmission.request.start <= self.now
                && transmission.request.end > self.now
                && transmission.request.frame.protocol == protocol
                && transmission.request.frame.spectrum.overlaps(spectrum)
                && self.received_power(transmission, receiver) >= minimum_power_dbm
        })
    }

    /// Submits a transmission without performing any host I/O.
    pub fn transmit(&mut self, request: TxRequest) -> Result<TransmissionId, MediumError> {
        if request.frame.spectrum.bandwidth_khz == 0 {
            return Err(MediumError::InvalidTransmissionBandwidth);
        }
        if request.frame.phy.is_empty() {
            return Err(MediumError::MissingPhy);
        }
        if request.end <= request.start {
            return Err(MediumError::InvalidInterval);
        }
        if request.start < self.now {
            return Err(MediumError::StartsInPast {
                start: request.start,
                now: self.now,
            });
        }
        let id = TransmissionId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(MediumError::IdentifierExhausted)?;
        self.events.push(MediumEvent::Submitted {
            id,
            request: request.clone(),
        });
        self.transmissions.push(PendingTransmission {
            id,
            request,
            resolved: false,
        });
        Ok(id)
    }

    /// Advances time and resolves all transmissions ending by `requested`.
    pub fn advance_to(&mut self, requested: SimTime) -> Result<(), MediumError> {
        if requested < self.now {
            return Err(MediumError::TimeReversal {
                now: self.now,
                requested,
            });
        }
        self.now = requested;
        let resolvable: Vec<usize> = self
            .transmissions
            .iter()
            .enumerate()
            .filter_map(|(index, transmission)| {
                (!transmission.resolved && transmission.request.end <= requested).then_some(index)
            })
            .collect();
        for index in resolvable {
            self.resolve(index);
            self.transmissions[index].resolved = true;
        }
        self.prune_history();
        Ok(())
    }

    /// Append-only event stream suitable for diagnostics and replay capture.
    pub fn events(&self) -> &[MediumEvent] {
        &self.events
    }

    /// Removes and returns all events emitted so far.
    pub fn drain_events(&mut self) -> Vec<MediumEvent> {
        core::mem::take(&mut self.events)
    }

    fn resolve(&mut self, index: usize) {
        let desired = self.transmissions[index].clone();
        let receivers: Vec<Receiver> = self
            .receivers
            .values()
            .filter(|receiver| {
                receiver.node != desired.request.source
                    && receiver.protocol == desired.request.frame.protocol
                    && receiver.spectrum.overlaps(desired.request.frame.spectrum)
            })
            .cloned()
            .collect();
        for receiver in receivers {
            let signal_dbm = self.received_power(&desired, receiver.node);
            let outcome = if signal_dbm < receiver.sensitivity_dbm {
                DeliveryOutcome::BelowSensitivity { signal_dbm }
            } else if let Some((interferer, interference_dbm)) =
                self.strongest_interferer(&desired, receiver.node)
            {
                if signal_dbm.saturating_sub(interference_dbm) >= self.profile.capture_threshold_db
                {
                    self.loss_or_delivery(desired.id, receiver.node)
                } else {
                    DeliveryOutcome::Collision {
                        interferer,
                        interference_dbm,
                    }
                }
            } else {
                self.loss_or_delivery(desired.id, receiver.node)
            };
            self.events.push(MediumEvent::Reception {
                id: desired.id,
                receiver: receiver.node,
                outcome,
            });
        }
    }

    fn strongest_interferer(
        &self,
        desired: &PendingTransmission,
        receiver: NodeId,
    ) -> Option<(TransmissionId, i16)> {
        self.transmissions
            .iter()
            .filter(|candidate| {
                candidate.id != desired.id
                    && intervals_overlap(
                        desired.request.start,
                        desired.request.end,
                        candidate.request.start,
                        candidate.request.end,
                    )
                    && desired
                        .request
                        .frame
                        .spectrum
                        .overlaps(candidate.request.frame.spectrum)
            })
            .map(|candidate| (candidate.id, self.received_power(candidate, receiver)))
            .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
    }

    fn received_power(&self, transmission: &PendingTransmission, receiver: NodeId) -> i16 {
        self.received_power_dbm(
            transmission.request.source,
            receiver,
            transmission.request.power_dbm,
        )
    }

    fn loss_or_delivery(&self, transmission: TransmissionId, receiver: NodeId) -> DeliveryOutcome {
        if self.profile.loss_ppm == 0 {
            return DeliveryOutcome::Delivered;
        }
        let sample = splitmix64(
            self.profile.seed
                ^ transmission.0.rotate_left(17)
                ^ u64::from(receiver.0).rotate_left(41),
        ) % 1_000_000;
        if sample < u64::from(self.profile.loss_ppm) {
            DeliveryOutcome::SeededLoss
        } else {
            DeliveryOutcome::Delivered
        }
    }

    fn prune_history(&mut self) {
        let earliest_unresolved_start = self
            .transmissions
            .iter()
            .filter(|transmission| !transmission.resolved)
            .map(|transmission| transmission.request.start)
            .min();
        self.transmissions.retain(|transmission| {
            !transmission.resolved
                || earliest_unresolved_start.is_some_and(|start| transmission.request.end > start)
        });
    }
}

fn ordered_node_pair(first: NodeId, second: NodeId) -> (NodeId, NodeId) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn intervals_overlap(
    first_start: SimTime,
    first_end: SimTime,
    second_start: SimTime,
    second_end: SimTime,
) -> bool {
    first_start < second_end && second_start < first_end
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameOrigin, RadioFrame, Spectrum};

    const WIFI: Spectrum = Spectrum::new(2_412_000, 20_000);

    fn frame(protocol: RadioProtocol, spectrum: Spectrum, bytes: &[u8]) -> RadioFrame {
        RadioFrame {
            protocol,
            spectrum,
            phy: "test-phy".to_owned(),
            bytes: bytes.to_vec(),
            origin: FrameOrigin::Emulated,
        }
    }

    fn request(source: u32, start: u64, end: u64, power: i16, bytes: &[u8]) -> TxRequest {
        TxRequest {
            source: NodeId(source),
            start: SimTime::from_ticks(start),
            end: SimTime::from_ticks(end),
            power_dbm: power,
            frame: frame(RadioProtocol::Wifi, WIFI, bytes),
        }
    }

    fn receiver(node: u32) -> Receiver {
        Receiver {
            node: NodeId(node),
            protocol: RadioProtocol::Wifi,
            spectrum: WIFI,
            sensitivity_dbm: -90,
        }
    }

    #[test]
    fn isolated_transmission_is_delivered() {
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.register_receiver(receiver(2)).unwrap();
        medium.transmit(request(1, 10, 20, 10, b"hello")).unwrap();
        medium.advance_to(SimTime::from_ticks(20)).unwrap();
        assert!(medium.events().iter().any(|event| matches!(
            event,
            MediumEvent::Reception {
                receiver: NodeId(2),
                outcome: DeliveryOutcome::Delivered,
                ..
            }
        )));
    }

    #[test]
    fn equal_power_overlap_collides_across_protocols() {
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.register_receiver(receiver(3)).unwrap();
        medium.transmit(request(1, 10, 30, 10, b"wifi")).unwrap();
        let mut ble = request(2, 15, 25, 10, b"ble");
        ble.frame.protocol = RadioProtocol::BluetoothLe;
        ble.frame.spectrum = Spectrum::new(2_412_000, 2_000);
        medium.transmit(ble).unwrap();
        medium.advance_to(SimTime::from_ticks(30)).unwrap();
        assert!(medium.events().iter().any(|event| matches!(
            event,
            MediumEvent::Reception {
                receiver: NodeId(3),
                outcome: DeliveryOutcome::Collision { .. },
                ..
            }
        )));
    }

    #[test]
    fn strong_signal_captures_weak_interferer() {
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.register_receiver(receiver(3)).unwrap();
        medium.set_path_loss(NodeId(1), NodeId(3), 20);
        medium.set_path_loss(NodeId(2), NodeId(3), 60);
        medium.transmit(request(1, 10, 30, 10, b"strong")).unwrap();
        medium.transmit(request(2, 15, 25, 10, b"weak")).unwrap();
        medium.advance_to(SimTime::from_ticks(30)).unwrap();
        assert!(medium.events().iter().any(|event| matches!(
            event,
            MediumEvent::Reception {
                id: TransmissionId(0),
                receiver: NodeId(3),
                outcome: DeliveryOutcome::Delivered,
            }
        )));
    }

    #[test]
    fn loss_is_repeatable_and_polling_independent() {
        let profile = MediumProfile {
            seed: 0x1234,
            loss_ppm: 500_000,
            ..MediumProfile::default()
        };
        let run = |incremental: bool| {
            let mut medium = RadioMedium::new(profile.clone()).unwrap();
            medium.register_receiver(receiver(7)).unwrap();
            for byte in 0_u8..32 {
                let index = u64::from(byte);
                medium
                    .transmit(request(1, index * 10, index * 10 + 5, 10, &[byte]))
                    .unwrap();
                if incremental {
                    medium
                        .advance_to(SimTime::from_ticks(index * 10 + 5))
                        .unwrap();
                }
            }
            medium.advance_to(SimTime::from_ticks(320)).unwrap();
            medium
                .events()
                .iter()
                .filter(|event| matches!(event, MediumEvent::Reception { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(run(false), run(true));
    }

    #[test]
    fn rejects_implicit_time_travel() {
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.advance_to(SimTime::from_ticks(10)).unwrap();
        assert!(matches!(
            medium.transmit(request(1, 9, 12, 10, b"late")),
            Err(MediumError::StartsInPast { .. })
        ));
        assert!(matches!(
            medium.advance_to(SimTime::from_ticks(9)),
            Err(MediumError::TimeReversal { .. })
        ));
    }
}
