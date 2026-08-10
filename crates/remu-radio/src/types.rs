use remu_core::SimTime;
use serde::{Deserialize, Serialize};

/// Wireless protocol carried by a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadioProtocol {
    /// IEEE 802.11 Wi-Fi.
    Wifi,
    /// Bluetooth Low Energy.
    BluetoothLe,
    /// IEEE 802.15.4 low-rate wireless PAN.
    Ieee802154,
}

/// Stable identifier for one emulated or host-side radio node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// Stable identifier allocated to a transmission in submission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransmissionId(pub u64);

/// Occupied radio spectrum expressed without floating-point arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spectrum {
    /// Center frequency in kHz.
    pub center_khz: u32,
    /// Total occupied bandwidth in kHz.
    pub bandwidth_khz: u32,
}

impl Spectrum {
    /// Creates a spectrum allocation.
    pub const fn new(center_khz: u32, bandwidth_khz: u32) -> Self {
        Self {
            center_khz,
            bandwidth_khz,
        }
    }

    /// Returns whether this allocation overlaps another allocation.
    pub fn overlaps(self, other: Self) -> bool {
        let self_low = u64::from(self.center_khz).saturating_sub(u64::from(self.bandwidth_khz) / 2);
        let self_high = u64::from(self.center_khz) + u64::from(self.bandwidth_khz).div_ceil(2);
        let other_low =
            u64::from(other.center_khz).saturating_sub(u64::from(other.bandwidth_khz) / 2);
        let other_high = u64::from(other.center_khz) + u64::from(other.bandwidth_khz).div_ceil(2);
        self_low < other_high && other_low < self_high
    }
}

/// How a frame entered the deterministic medium.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameOrigin {
    /// Submitted by an emulated radio peripheral.
    Emulated,
    /// Explicitly injected by a host API.
    HostInjection,
    /// Loaded from a replay artifact.
    Replay,
}

/// Protocol frame and its physical-layer metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioFrame {
    /// Protocol that interprets the packet bytes.
    pub protocol: RadioProtocol,
    /// Occupied RF spectrum.
    pub spectrum: Spectrum,
    /// Protocol-specific PHY name, such as `wifi-ht20` or `ble-1m`.
    pub phy: String,
    /// Uninterpreted physical-layer service data unit.
    pub bytes: Vec<u8>,
    /// Source of this frame.
    pub origin: FrameOrigin,
}

/// Request to occupy the RF medium for a bounded interval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRequest {
    /// Node transmitting the frame.
    pub source: NodeId,
    /// Inclusive transmission start timestamp.
    pub start: SimTime,
    /// Exclusive transmission end timestamp.
    pub end: SimTime,
    /// Transmit power in integer dBm.
    pub power_dbm: i16,
    /// Frame carried by the transmission.
    pub frame: RadioFrame,
}

/// A protocol receiver registered with the medium.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receiver {
    /// Node owning the receiver.
    pub node: NodeId,
    /// Protocol currently decoded by the receiver.
    pub protocol: RadioProtocol,
    /// Spectrum to which the receiver is tuned.
    pub spectrum: Spectrum,
    /// Weakest accepted signal in integer dBm.
    pub sensitivity_dbm: i16,
}

/// Deterministic RF behavior knobs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediumProfile {
    /// Seed mixed with stable transmission and receiver IDs.
    pub seed: u64,
    /// Independent packet loss probability in parts per million.
    pub loss_ppm: u32,
    /// Required desired-signal advantage for capture, in dB.
    pub capture_threshold_db: i16,
    /// Default path loss between nodes without an explicit link override.
    pub default_path_loss_db: i16,
}

impl Default for MediumProfile {
    fn default() -> Self {
        Self {
            seed: 0,
            loss_ppm: 0,
            capture_threshold_db: 10,
            default_path_loss_db: 40,
        }
    }
}

/// Final result for one candidate receiver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DeliveryOutcome {
    /// Packet was decoded and delivered.
    Delivered,
    /// Desired signal was below receiver sensitivity.
    BelowSensitivity {
        /// Received signal strength in integer dBm.
        signal_dbm: i16,
    },
    /// Another overlapping transmission prevented decoding.
    Collision {
        /// Strongest interfering transmission.
        interferer: TransmissionId,
        /// Received power of the strongest interferer in integer dBm.
        interference_dbm: i16,
    },
    /// Seeded loss profile discarded the otherwise decodable packet.
    SeededLoss,
}

/// Append-only event emitted by the medium and stored in replay artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum MediumEvent {
    /// A transmission was accepted by the medium.
    Submitted {
        /// Stable transmission identifier.
        id: TransmissionId,
        /// Full immutable request.
        request: TxRequest,
    },
    /// A transmission reached a compatible receiver.
    Reception {
        /// Stable transmission identifier.
        id: TransmissionId,
        /// Receiving node.
        receiver: NodeId,
        /// Deterministic result.
        outcome: DeliveryOutcome,
    },
}
