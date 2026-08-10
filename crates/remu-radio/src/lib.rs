//! Deterministic radio-medium contracts shared by emulated wireless `SoCs`.
//!
//! The crate deliberately has no socket or host-network dependency. Packets
//! enter and leave through explicit values so callers can record, replay, and
//! test every RF interaction.

mod ble;
mod coexistence;
mod ieee802154;
mod medium;
mod replay;
mod types;
mod wifi;

pub use ble::{BdAddress, BleController, BleError, BleEvent, BlePeer, BlePhy, ConnectionHandle};
pub use coexistence::{
    CoexistenceArbiter, CoexistenceDecision, CoexistenceError, CoexistenceEvent,
    CoexistenceGrantId, CoexistenceRequest,
};
pub use ieee802154::{
    ExtendedAddress, Ieee802154CcaMode, Ieee802154Error, Ieee802154Event, Ieee802154Mac,
    Ieee802154RxOutcome, PanInterface, SecurityMaterial, ShortAddress,
};
pub use medium::{MediumError, RadioMedium};
pub use replay::{ReplayArtifact, ReplayError};
pub use types::{
    DeliveryOutcome, FrameOrigin, MediumEvent, MediumProfile, NodeId, RadioFrame, RadioProtocol,
    Receiver, Spectrum, TransmissionId, TxRequest,
};
pub use wifi::{
    MacAddress, WifiEngine, WifiError, WifiEvent, WifiMode, WifiNetwork, WifiSecurity,
    WifiStationState,
};
