use crate::{FrameOrigin, RadioFrame, RadioProtocol, Spectrum};
use aes::Aes128;
use ccm::{
    Ccm,
    aead::{AeadInOut, KeyInit},
    consts::{U8, U13},
};
use remu_core::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use thiserror::Error;

/// Six-byte IEEE MAC address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// Broadcast destination address.
    pub const BROADCAST: Self = Self([0xff; 6]);

    /// Returns true for the broadcast address.
    pub fn is_broadcast(self) -> bool {
        self == Self::BROADCAST
    }

    /// Returns true for an IEEE group/multicast address.
    pub const fn is_multicast(self) -> bool {
        self.0[0] & 1 != 0
    }
}

/// Enabled Wi-Fi interface combination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiMode {
    /// Controller is stopped.
    #[default]
    Disabled,
    /// Infrastructure station.
    Station,
    /// Software access point.
    SoftAp,
    /// Concurrent station and software access point.
    StationAndSoftAp,
    /// Raw promiscuous receive mode.
    Monitor,
}

/// Network authentication/cipher policy exposed at the functional API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiSecurity {
    /// No link-layer authentication or encryption.
    Open,
    /// WPA2 personal using CCMP.
    Wpa2Personal,
    /// WPA3 personal using SAE and CCMP.
    Wpa3Personal,
    /// WPA2/WPA3 transition mode.
    Wpa2Wpa3Personal,
}

/// Deterministic network advertised by a scripted RF peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiNetwork {
    /// Service-set identifier, limited to 32 bytes by validation.
    pub ssid: String,
    /// Access-point MAC address.
    pub bssid: MacAddress,
    /// 2.4 GHz channel 1 through 14.
    pub channel: u8,
    /// Received signal strength in integer dBm.
    pub rssi_dbm: i16,
    /// Authentication/cipher policy.
    pub security: WifiSecurity,
    /// Whether the AP advertises Wi-Fi 6/802.11ax capability.
    pub wifi6: bool,
}

/// Software-access-point configuration used by the deterministic peer model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiSoftApConfiguration {
    /// Advertised service-set identifier.
    pub ssid: String,
    /// Authentication and cipher policy.
    pub security: WifiSecurity,
    /// Optional WPA passphrase; absent for an open network.
    pub passphrase: Option<String>,
    /// Maximum associated stations from one through sixteen.
    pub max_clients: u8,
    /// Beacon interval in 1.024 ms time units.
    pub beacon_interval: u16,
    /// Advertise an 802.11ax/HE capability element on C6-style peers.
    pub wifi6: bool,
}

/// One station associated with the functional `SoftAP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WifiSoftApClient {
    /// Station MAC address.
    pub address: MacAddress,
    /// Association identifier allocated by the AP.
    pub aid: u16,
    /// Whether the station currently requests buffered delivery.
    pub power_save: bool,
}

/// Infrastructure-station state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum WifiStationState {
    /// Controller or station interface is inactive.
    #[default]
    Stopped,
    /// Station is started but not associated.
    Idle,
    /// A deterministic scan is in progress.
    Scanning,
    /// Authenticated and associated to an AP.
    Associated {
        /// Associated BSSID.
        bssid: MacAddress,
        /// Association identifier assigned by the functional peer.
        aid: u16,
    },
}

/// Observable Wi-Fi engine transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum WifiEvent {
    /// Interface combination started.
    Started {
        /// Active mode.
        mode: WifiMode,
    },
    /// Controller stopped and queues were cleared.
    Stopped,
    /// Scan completed in deterministic channel/BSSID order.
    ScanDone {
        /// Number of matching networks.
        count: usize,
    },
    /// Station associated successfully.
    Associated {
        /// Peer BSSID.
        bssid: MacAddress,
        /// Assigned association identifier.
        aid: u16,
    },
    /// Station left its AP.
    Disconnected {
        /// Stable project-owned reason.
        reason: String,
    },
    /// A raw MAC frame was queued for transmission.
    TxQueued {
        /// Stable local queue sequence.
        sequence: u64,
        /// Frame length in bytes.
        length: usize,
    },
    /// A raw MAC frame passed channel/address filters.
    RxAccepted {
        /// Receiver address from the 802.11 header.
        destination: MacAddress,
        /// Frame length in bytes.
        length: usize,
    },
    /// A well-formed frame was rejected by filtering.
    RxFiltered {
        /// Receiver address from the 802.11 header.
        destination: MacAddress,
    },
    /// A malformed or too-short frame was rejected.
    RxMalformed,
    /// A periodic `SoftAP` beacon was queued.
    Beacon {
        /// Beacon timestamp in simulation ticks.
        timestamp: u64,
    },
    /// A station joined the functional `SoftAP`.
    SoftApClientAssociated {
        /// Station address.
        address: MacAddress,
        /// Allocated association identifier.
        aid: u16,
    },
    /// A station left the functional `SoftAP`.
    SoftApClientDisconnected {
        /// Station address.
        address: MacAddress,
    },
    /// An ESP-NOW vendor action frame was queued.
    EspNowQueued {
        /// Destination peer.
        destination: MacAddress,
        /// Application payload length.
        length: usize,
    },
    /// A transmit attempt completed or was requeued.
    TxCompleted {
        /// Stable local queue sequence.
        sequence: u64,
        /// True when the peer acknowledged the frame.
        acknowledged: bool,
        /// Number of attempts made so far.
        attempts: u8,
    },
}

/// Wi-Fi configuration or packet error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WifiError {
    /// SSID exceeds the IEEE maximum of 32 bytes.
    #[error("SSID is {0} bytes; maximum is 32")]
    SsidTooLong(usize),
    /// Only 2.4 GHz channels 1 through 14 are modeled.
    #[error("invalid 2.4 GHz Wi-Fi channel {0}")]
    InvalidChannel(u8),
    /// Operation requires a started interface.
    #[error("Wi-Fi controller is stopped")]
    Stopped,
    /// Operation requires station mode.
    #[error("Wi-Fi station interface is not enabled")]
    StationDisabled,
    /// Operation requires software-AP mode.
    #[error("Wi-Fi software AP interface is not enabled")]
    SoftApDisabled,
    /// Requested scripted network does not exist.
    #[error("network not found")]
    NetworkNotFound,
    /// Credentials do not satisfy the network's security policy.
    #[error("authentication failed")]
    AuthenticationFailed,
    /// Raw 802.11 frame is shorter than its required header.
    #[error("malformed 802.11 frame")]
    MalformedFrame,
    /// Stable queue identifiers have been exhausted.
    #[error("Wi-Fi transmit sequence exhausted")]
    SequenceExhausted,
    /// Software-AP configuration is internally inconsistent.
    #[error("invalid Wi-Fi software AP configuration")]
    InvalidSoftApConfiguration,
    /// Software AP has reached its configured association capacity.
    #[error("Wi-Fi software AP client table is full")]
    SoftApFull,
    /// ESP-NOW payload exceeds the legacy 250-byte maximum.
    #[error("ESP-NOW payload is {0} bytes; maximum is 250")]
    EspNowPayloadTooLong(usize),
    /// Completion named a frame that is not awaiting an ACK result.
    #[error("unknown Wi-Fi transmit sequence {0}")]
    UnknownTransmission(u64),
    /// A native CCMP frame or header does not satisfy the recovered hardware format.
    #[error("malformed Wi-Fi CCMP frame")]
    MalformedCcmpFrame,
    /// A CCMP authentication tag did not verify under the selected hardware key.
    #[error("Wi-Fi CCMP authentication failed")]
    CcmpAuthenticationFailed,
}

/// Applies AES-CCM to a native, preformatted CCMP transmit frame in place.
///
/// Guest net80211 has already set Protected Frame, inserted the eight-byte
/// CCMP header, and reserved the final eight bytes for the hardware MIC. The
/// MAC encrypts only the bytes between those regions and replaces the MIC
/// reservation. This matches the boundary used by C6 and S3 vendor LMAC.
pub fn protect_native_ccmp_frame(key: &[u8; 16], frame: &mut [u8]) -> Result<(), WifiError> {
    let (header_length, aad, nonce) = ccmp_aad_nonce(frame)?;
    let payload_start = header_length + 8;
    let payload_end = frame
        .len()
        .checked_sub(8)
        .filter(|end| *end >= payload_start)
        .ok_or(WifiError::MalformedCcmpFrame)?;
    let cipher = Ccm::<Aes128, U8, U13>::new(key.into());
    let tag = cipher
        .encrypt_inout_detached(
            (&nonce).into(),
            &aad,
            (&mut frame[payload_start..payload_end]).into(),
        )
        .map_err(|_| WifiError::CcmpAuthenticationFailed)?;
    frame[payload_end..].copy_from_slice(&tag);
    Ok(())
}

/// Authenticates and decrypts a native CCMP receive frame in place.
pub fn unprotect_native_ccmp_frame(key: &[u8; 16], frame: &mut [u8]) -> Result<(), WifiError> {
    let (header_length, aad, nonce) = ccmp_aad_nonce(frame)?;
    let payload_start = header_length + 8;
    let payload_end = frame
        .len()
        .checked_sub(8)
        .filter(|end| *end >= payload_start)
        .ok_or(WifiError::MalformedCcmpFrame)?;
    let tag: [u8; 8] = frame[payload_end..]
        .try_into()
        .map_err(|_| WifiError::MalformedCcmpFrame)?;
    let cipher = Ccm::<Aes128, U8, U13>::new(key.into());
    cipher
        .decrypt_inout_detached(
            (&nonce).into(),
            &aad,
            (&mut frame[payload_start..payload_end]).into(),
            (&tag).into(),
        )
        .map_err(|_| WifiError::CcmpAuthenticationFailed)
}

fn ccmp_aad_nonce(frame: &[u8]) -> Result<(usize, Vec<u8>, [u8; 13]), WifiError> {
    let frame_control = u16::from_le_bytes(
        frame
            .get(..2)
            .ok_or(WifiError::MalformedCcmpFrame)?
            .try_into()
            .expect("checked frame-control width"),
    );
    let frame_type = frame_control & 0x000c;
    if !matches!(frame_type, 0x0000 | 0x0008) || frame_control & 0x4000 == 0 {
        return Err(WifiError::MalformedCcmpFrame);
    }
    let address_four = frame_control & 0x0300 == 0x0300;
    let qos = frame_type == 0x0008 && frame_control & 0x0080 != 0;
    let high_throughput_control = qos && frame_control & 0x8000 != 0;
    let header_length = 24
        + usize::from(address_four) * 6
        + usize::from(qos) * 2
        + usize::from(high_throughput_control) * 4;
    let ccmp = frame
        .get(header_length..header_length + 8)
        .ok_or(WifiError::MalformedCcmpFrame)?;
    if ccmp[2] != 0 || ccmp[3] & 0x3f != 0x20 {
        return Err(WifiError::MalformedCcmpFrame);
    }

    let mut authenticated_control = frame_control;
    if frame_type == 0x0008 {
        authenticated_control &= !0x0070;
        if qos {
            authenticated_control &= !0x8000;
        }
    }
    authenticated_control &= !(0x0800 | 0x1000 | 0x2000);
    authenticated_control |= 0x4000;

    let mut aad = Vec::with_capacity(30);
    aad.extend_from_slice(&authenticated_control.to_le_bytes());
    aad.extend_from_slice(frame.get(4..22).ok_or(WifiError::MalformedCcmpFrame)?);
    let sequence_control = u16::from_le_bytes(
        frame
            .get(22..24)
            .ok_or(WifiError::MalformedCcmpFrame)?
            .try_into()
            .expect("checked sequence-control width"),
    ) & 0x000f;
    aad.extend_from_slice(&sequence_control.to_le_bytes());
    let mut extension = 24;
    if address_four {
        aad.extend_from_slice(
            frame
                .get(extension..extension + 6)
                .ok_or(WifiError::MalformedCcmpFrame)?,
        );
        extension += 6;
    }
    let priority = if qos {
        let quality = frame
            .get(extension..extension + 2)
            .ok_or(WifiError::MalformedCcmpFrame)?;
        aad.extend_from_slice(&[quality[0] & 0x0f, 0]);
        quality[0] & 0x0f
    } else if frame_type == 0 {
        0x10
    } else {
        0
    };

    let mut nonce = [0_u8; 13];
    nonce[0] = priority;
    nonce[1..7].copy_from_slice(frame.get(10..16).ok_or(WifiError::MalformedCcmpFrame)?);
    nonce[7..].copy_from_slice(&[ccmp[7], ccmp[6], ccmp[5], ccmp[4], ccmp[1], ccmp[0]]);
    Ok((header_length, aad, nonce))
}

#[derive(Clone, Debug)]
struct QueuedWifiFrame {
    sequence: u64,
    frame: RadioFrame,
    attempts: u8,
    max_retries: u8,
}

/// Deterministic functional 2.4 GHz Wi-Fi MAC/state engine.
#[derive(Clone, Debug)]
pub struct WifiEngine {
    mac: MacAddress,
    mode: WifiMode,
    channel: u8,
    station: WifiStationState,
    networks: BTreeMap<(u8, MacAddress), WifiNetwork>,
    scan_results: Vec<WifiNetwork>,
    tx: VecDeque<QueuedWifiFrame>,
    awaiting_tx: BTreeMap<u64, QueuedWifiFrame>,
    rx: VecDeque<Vec<u8>>,
    events: Vec<WifiEvent>,
    next_sequence: u64,
    power_save: bool,
    soft_ap: Option<WifiSoftApConfiguration>,
    soft_ap_clients: BTreeMap<MacAddress, WifiSoftApClient>,
    now: SimTime,
    next_beacon: Option<SimTime>,
}

impl WifiEngine {
    /// Creates a stopped controller with a stable local MAC address.
    pub fn new(mac: MacAddress) -> Self {
        Self {
            mac,
            mode: WifiMode::Disabled,
            channel: 1,
            station: WifiStationState::Stopped,
            networks: BTreeMap::new(),
            scan_results: Vec::new(),
            tx: VecDeque::new(),
            awaiting_tx: BTreeMap::new(),
            rx: VecDeque::new(),
            events: Vec::new(),
            next_sequence: 0,
            power_save: false,
            soft_ap: None,
            soft_ap_clients: BTreeMap::new(),
            now: SimTime::ZERO,
            next_beacon: None,
        }
    }

    /// Local interface MAC address.
    pub const fn mac(&self) -> MacAddress {
        self.mac
    }

    /// Active interface mode.
    pub const fn mode(&self) -> WifiMode {
        self.mode
    }

    /// Current RF channel.
    pub const fn channel(&self) -> u8 {
        self.channel
    }

    /// Current station state.
    pub const fn station_state(&self) -> &WifiStationState {
        &self.station
    }

    /// Enables or disables functional station power-save state.
    pub fn set_power_save(&mut self, enabled: bool) -> Result<(), WifiError> {
        self.require_started()?;
        self.power_save = enabled;
        Ok(())
    }

    /// Returns the functional power-save setting.
    pub const fn power_save(&self) -> bool {
        self.power_save
    }

    /// Starts the requested interface combination.
    pub fn start(&mut self, mode: WifiMode) -> Result<(), WifiError> {
        if mode == WifiMode::Disabled {
            return self.stop();
        }
        self.mode = mode;
        self.station = if has_station(mode) {
            WifiStationState::Idle
        } else {
            WifiStationState::Stopped
        };
        self.events.push(WifiEvent::Started { mode });
        self.next_beacon = (has_soft_ap(mode) && self.soft_ap.is_some()).then_some(self.now);
        Ok(())
    }

    /// Stops all interfaces and clears volatile packet queues.
    pub fn stop(&mut self) -> Result<(), WifiError> {
        self.mode = WifiMode::Disabled;
        self.station = WifiStationState::Stopped;
        self.scan_results.clear();
        self.tx.clear();
        self.awaiting_tx.clear();
        self.rx.clear();
        self.soft_ap_clients.clear();
        self.next_beacon = None;
        self.power_save = false;
        self.events.push(WifiEvent::Stopped);
        Ok(())
    }

    /// Selects a 2.4 GHz channel.
    pub fn set_channel(&mut self, channel: u8) -> Result<(), WifiError> {
        validate_channel(channel)?;
        self.require_started()?;
        self.channel = channel;
        Ok(())
    }

    /// Adds or replaces a scripted access point visible to scans.
    pub fn add_network(&mut self, network: WifiNetwork) -> Result<(), WifiError> {
        validate_ssid(&network.ssid)?;
        validate_channel(network.channel)?;
        self.networks
            .insert((network.channel, network.bssid), network);
        Ok(())
    }

    /// Removes all scripted RF peers.
    pub fn clear_networks(&mut self) {
        self.networks.clear();
        self.scan_results.clear();
    }

    /// Performs a deterministic scan, optionally limited to one channel.
    pub fn scan(&mut self, channel: Option<u8>) -> Result<&[WifiNetwork], WifiError> {
        self.require_station()?;
        if let Some(channel) = channel {
            validate_channel(channel)?;
        }
        self.station = WifiStationState::Scanning;
        self.scan_results = self
            .networks
            .values()
            .filter(|network| channel.is_none_or(|selected| network.channel == selected))
            .cloned()
            .collect();
        self.station = WifiStationState::Idle;
        self.events.push(WifiEvent::ScanDone {
            count: self.scan_results.len(),
        });
        Ok(&self.scan_results)
    }

    /// Associates to a scripted peer after deterministic credential validation.
    pub fn associate(
        &mut self,
        bssid: MacAddress,
        passphrase: Option<&str>,
    ) -> Result<(), WifiError> {
        self.require_station()?;
        let network = self
            .networks
            .values()
            .find(|network| network.bssid == bssid)
            .ok_or(WifiError::NetworkNotFound)?;
        let authenticated = match network.security {
            WifiSecurity::Open => passphrase.is_none_or(str::is_empty),
            WifiSecurity::Wpa2Personal
            | WifiSecurity::Wpa3Personal
            | WifiSecurity::Wpa2Wpa3Personal => passphrase.is_some_and(|key| {
                let length = key.len();
                (8..=63).contains(&length)
            }),
        };
        if !authenticated {
            return Err(WifiError::AuthenticationFailed);
        }
        self.channel = network.channel;
        let aid = deterministic_aid(bssid);
        self.station = WifiStationState::Associated { bssid, aid };
        self.events.push(WifiEvent::Associated { bssid, aid });
        Ok(())
    }

    /// Leaves the current AP and returns to station idle.
    pub fn disconnect(&mut self, reason: impl Into<String>) -> Result<(), WifiError> {
        self.require_station()?;
        self.station = WifiStationState::Idle;
        self.events.push(WifiEvent::Disconnected {
            reason: reason.into(),
        });
        Ok(())
    }

    /// Queues a raw 802.11 MAC frame for the shared medium.
    pub fn queue_tx(&mut self, bytes: Vec<u8>) -> Result<u64, WifiError> {
        self.queue_tx_with_retries(bytes, 0)
    }

    /// Queues a raw MAC frame with deterministic ACK/retry policy.
    pub fn queue_tx_with_retries(
        &mut self,
        bytes: Vec<u8>,
        max_retries: u8,
    ) -> Result<u64, WifiError> {
        self.require_started()?;
        parse_receiver_address(&bytes)?;
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(WifiError::SequenceExhausted)?;
        let length = bytes.len();
        self.tx.push_back(QueuedWifiFrame {
            sequence,
            frame: RadioFrame {
                protocol: RadioProtocol::Wifi,
                spectrum: wifi_spectrum(self.channel),
                phy: "wifi-ht20".to_owned(),
                bytes,
                mpdus: Vec::new(),
                origin: FrameOrigin::Emulated,
            },
            attempts: 0,
            max_retries,
        });
        self.events.push(WifiEvent::TxQueued { sequence, length });
        Ok(sequence)
    }

    /// Removes the oldest raw frame waiting for medium submission.
    pub fn take_tx(&mut self) -> Option<(u64, RadioFrame)> {
        let mut queued = self.tx.pop_front()?;
        queued.attempts = queued.attempts.saturating_add(1);
        let result = (queued.sequence, queued.frame.clone());
        if queued.max_retries != 0 {
            self.awaiting_tx.insert(queued.sequence, queued);
        }
        Some(result)
    }

    /// Completes one transmit attempt, requeueing an unacknowledged frame when
    /// its retry budget has not been exhausted.
    pub fn complete_tx(&mut self, sequence: u64, acknowledged: bool) -> Result<bool, WifiError> {
        let queued = self
            .awaiting_tx
            .remove(&sequence)
            .ok_or(WifiError::UnknownTransmission(sequence))?;
        let retry = !acknowledged && queued.attempts <= queued.max_retries;
        self.events.push(WifiEvent::TxCompleted {
            sequence,
            acknowledged,
            attempts: queued.attempts,
        });
        if retry {
            self.tx.push_front(queued);
        }
        Ok(retry)
    }

    /// Configures beaconing and association policy for `SoftAP` modes.
    pub fn configure_soft_ap(
        &mut self,
        configuration: WifiSoftApConfiguration,
    ) -> Result<(), WifiError> {
        validate_ssid(&configuration.ssid)?;
        let valid_credentials = match configuration.security {
            WifiSecurity::Open => configuration
                .passphrase
                .as_deref()
                .is_none_or(str::is_empty),
            _ => configuration
                .passphrase
                .as_ref()
                .is_some_and(|passphrase| (8..=63).contains(&passphrase.len())),
        };
        if !valid_credentials
            || !(1..=16).contains(&configuration.max_clients)
            || configuration.beacon_interval == 0
        {
            return Err(WifiError::InvalidSoftApConfiguration);
        }
        self.soft_ap = Some(configuration);
        self.soft_ap_clients.clear();
        self.next_beacon = has_soft_ap(self.mode).then_some(self.now);
        Ok(())
    }

    /// Associates one deterministic station with the configured `SoftAP`.
    pub fn associate_soft_ap_client(
        &mut self,
        address: MacAddress,
    ) -> Result<WifiSoftApClient, WifiError> {
        self.require_soft_ap()?;
        let configuration = self
            .soft_ap
            .as_ref()
            .ok_or(WifiError::InvalidSoftApConfiguration)?;
        if let Some(client) = self.soft_ap_clients.get(&address) {
            return Ok(*client);
        }
        if self.soft_ap_clients.len() >= usize::from(configuration.max_clients) {
            return Err(WifiError::SoftApFull);
        }
        let client = WifiSoftApClient {
            address,
            aid: deterministic_aid(address),
            power_save: false,
        };
        self.soft_ap_clients.insert(address, client);
        self.events.push(WifiEvent::SoftApClientAssociated {
            address,
            aid: client.aid,
        });
        Ok(client)
    }

    /// Removes one functional `SoftAP` station.
    pub fn disconnect_soft_ap_client(&mut self, address: MacAddress) -> Result<bool, WifiError> {
        self.require_soft_ap()?;
        let removed = self.soft_ap_clients.remove(&address).is_some();
        if removed {
            self.events
                .push(WifiEvent::SoftApClientDisconnected { address });
        }
        Ok(removed)
    }

    /// Current deterministic `SoftAP` association table.
    pub fn soft_ap_clients(&self) -> impl Iterator<Item = &WifiSoftApClient> {
        self.soft_ap_clients.values()
    }

    /// Queues a standards-shaped ESP-NOW vendor action frame.
    pub fn queue_esp_now(
        &mut self,
        destination: MacAddress,
        payload: &[u8],
    ) -> Result<u64, WifiError> {
        if payload.len() > 250 {
            return Err(WifiError::EspNowPayloadTooLong(payload.len()));
        }
        let mut frame = vec![0xd0, 0x00, 0, 0];
        frame.extend_from_slice(&destination.0);
        frame.extend_from_slice(&self.mac.0);
        frame.extend_from_slice(&MacAddress::BROADCAST.0);
        frame.extend_from_slice(&[0, 0, 127, 0x18, 0xfe, 0x34]);
        frame.extend_from_slice(&self.next_sequence.to_le_bytes()[..4]);
        frame.extend_from_slice(&[
            221,
            u8::try_from(payload.len() + 5).unwrap(),
            0x18,
            0xfe,
            0x34,
            4,
            1,
        ]);
        frame.extend_from_slice(payload);
        let sequence = self.queue_tx_with_retries(frame, 3)?;
        self.events.push(WifiEvent::EspNowQueued {
            destination,
            length: payload.len(),
        });
        Ok(sequence)
    }

    /// Advances TSF-derived `SoftAP` beacon scheduling.
    pub fn advance_to(&mut self, now: SimTime) {
        if now < self.now {
            return;
        }
        self.now = now;
        if has_soft_ap(self.mode)
            && self.soft_ap.is_some()
            && self.next_beacon.is_none_or(|deadline| deadline <= now)
        {
            let beacon = self.make_beacon();
            let _ = self.queue_tx(beacon);
            self.events.push(WifiEvent::Beacon {
                timestamp: now.ticks(),
            });
            let interval = self
                .soft_ap
                .as_ref()
                .expect("checked SoftAP configuration")
                .beacon_interval;
            self.next_beacon = now
                .checked_add(remu_core::SimDuration::from_ticks(
                    u64::from(interval) * 1_024,
                ))
                .ok();
        }
    }

    /// Applies channel and destination filters to an incoming raw frame.
    pub fn receive(&mut self, frame: &RadioFrame) -> Result<bool, WifiError> {
        self.require_started()?;
        if frame.protocol != RadioProtocol::Wifi
            || !frame.spectrum.overlaps(wifi_spectrum(self.channel))
        {
            return Ok(false);
        }
        let destination = match parse_receiver_address(&frame.bytes) {
            Ok(address) => address,
            Err(error) => {
                self.events.push(WifiEvent::RxMalformed);
                return Err(error);
            }
        };
        let accepted = self.mode == WifiMode::Monitor
            || destination == self.mac
            || destination.is_broadcast()
            || destination.is_multicast();
        if accepted {
            self.rx.push_back(frame.bytes.clone());
            self.events.push(WifiEvent::RxAccepted {
                destination,
                length: frame.bytes.len(),
            });
        } else {
            self.events.push(WifiEvent::RxFiltered { destination });
        }
        Ok(accepted)
    }

    /// Removes the oldest accepted raw receive frame.
    pub fn take_rx(&mut self) -> Option<Vec<u8>> {
        self.rx.pop_front()
    }

    /// Returns whether an accepted receive frame is waiting for the host stack.
    pub fn has_rx(&self) -> bool {
        !self.rx.is_empty()
    }

    /// Ordered scan results from the most recent scan.
    pub fn scan_results(&self) -> &[WifiNetwork] {
        &self.scan_results
    }

    /// Append-only state/packet event evidence.
    pub fn events(&self) -> &[WifiEvent] {
        &self.events
    }

    /// Applies a complete controller reset, preserving scripted RF peers.
    pub fn reset(&mut self) {
        self.mode = WifiMode::Disabled;
        self.channel = 1;
        self.station = WifiStationState::Stopped;
        self.scan_results.clear();
        self.tx.clear();
        self.awaiting_tx.clear();
        self.rx.clear();
        self.events.clear();
        self.power_save = false;
        self.soft_ap_clients.clear();
        self.next_beacon = None;
    }

    fn require_started(&self) -> Result<(), WifiError> {
        if self.mode == WifiMode::Disabled {
            Err(WifiError::Stopped)
        } else {
            Ok(())
        }
    }

    fn require_station(&self) -> Result<(), WifiError> {
        self.require_started()?;
        if has_station(self.mode) {
            Ok(())
        } else {
            Err(WifiError::StationDisabled)
        }
    }

    fn require_soft_ap(&self) -> Result<(), WifiError> {
        self.require_started()?;
        if has_soft_ap(self.mode) {
            Ok(())
        } else {
            Err(WifiError::SoftApDisabled)
        }
    }

    fn make_beacon(&self) -> Vec<u8> {
        let configuration = self.soft_ap.as_ref().expect("configured SoftAP beacon");
        let mut frame = vec![0x80, 0x00, 0, 0];
        frame.extend_from_slice(&MacAddress::BROADCAST.0);
        frame.extend_from_slice(&self.mac.0);
        frame.extend_from_slice(&self.mac.0);
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&self.now.ticks().to_le_bytes());
        frame.extend_from_slice(&configuration.beacon_interval.to_le_bytes());
        frame.extend_from_slice(&0x0001_u16.to_le_bytes());
        frame.extend_from_slice(&[
            0,
            u8::try_from(configuration.ssid.len()).expect("validated SSID length"),
        ]);
        frame.extend_from_slice(configuration.ssid.as_bytes());
        frame.extend_from_slice(&[3, 1, self.channel]);
        if configuration.security != WifiSecurity::Open {
            frame.extend_from_slice(&[48, 2, 1, 0]);
        }
        if configuration.wifi6 {
            frame.extend_from_slice(&[255, 2, 35, 1]);
        }
        frame
    }
}

fn has_station(mode: WifiMode) -> bool {
    matches!(mode, WifiMode::Station | WifiMode::StationAndSoftAp)
}

fn has_soft_ap(mode: WifiMode) -> bool {
    matches!(mode, WifiMode::SoftAp | WifiMode::StationAndSoftAp)
}

fn validate_channel(channel: u8) -> Result<(), WifiError> {
    if (1..=14).contains(&channel) {
        Ok(())
    } else {
        Err(WifiError::InvalidChannel(channel))
    }
}

fn validate_ssid(ssid: &str) -> Result<(), WifiError> {
    if ssid.len() <= 32 {
        Ok(())
    } else {
        Err(WifiError::SsidTooLong(ssid.len()))
    }
}

fn wifi_spectrum(channel: u8) -> Spectrum {
    let center_khz = if channel == 14 {
        2_484_000
    } else {
        2_412_000 + u32::from(channel - 1) * 5_000
    };
    Spectrum::new(center_khz, 20_000)
}

fn parse_receiver_address(frame: &[u8]) -> Result<MacAddress, WifiError> {
    if frame.len() < 24 {
        return Err(WifiError::MalformedFrame);
    }
    let bytes: [u8; 6] = frame
        .get(4..10)
        .ok_or(WifiError::MalformedFrame)?
        .try_into()
        .expect("checked six-byte receiver address");
    Ok(MacAddress(bytes))
}

fn deterministic_aid(bssid: MacAddress) -> u16 {
    let mixed = bssid
        .0
        .into_iter()
        .fold(0_u16, |value, byte| value.rotate_left(3) ^ u16::from(byte));
    (mixed % 2_007) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: MacAddress = MacAddress([0x02, 0, 0, 0, 0, 1]);
    const AP: MacAddress = MacAddress([0x02, 0, 0, 0, 0, 2]);

    fn network() -> WifiNetwork {
        WifiNetwork {
            ssid: "remu-net".to_owned(),
            bssid: AP,
            channel: 6,
            rssi_dbm: -42,
            security: WifiSecurity::Wpa2Personal,
            wifi6: true,
        }
    }

    fn data_frame(destination: MacAddress) -> Vec<u8> {
        let mut frame = vec![0x08, 0x01, 0, 0];
        frame.extend_from_slice(&destination.0);
        frame.extend_from_slice(&AP.0);
        frame.extend_from_slice(&AP.0);
        frame.extend_from_slice(&[0, 0, 1, 2, 3]);
        frame
    }

    fn native_ccmp_qos_frame() -> Vec<u8> {
        let mut frame = vec![0x88, 0x41, 0, 0];
        frame.extend_from_slice(&[0x02, 0x52, 0x45, 0x4d, 0x55, 0x01]);
        frame.extend_from_slice(&LOCAL.0);
        frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
        frame.extend_from_slice(&[0x30, 0x12]);
        frame.extend_from_slice(&[5, 0]);
        frame.extend_from_slice(&[1, 2, 0, 0x60, 3, 4, 5, 6]);
        frame.extend_from_slice(b"hello CCMP");
        frame.extend_from_slice(&[0; 8]);
        frame
    }

    #[test]
    fn native_ccmp_matches_an_independent_aes_ccm_vector_and_round_trips() {
        let key: [u8; 16] = core::array::from_fn(|index| index as u8);
        let mut frame = native_ccmp_qos_frame();
        protect_native_ccmp_frame(&key, &mut frame).unwrap();
        assert_eq!(
            hex::encode(&frame),
            "884100000252454d550102000000000102aabbccddee30120500\
             010200600304050676080b765c08082ec6e32f1e9b99e139c901"
                .replace(' ', "")
        );
        unprotect_native_ccmp_frame(&key, &mut frame).unwrap();
        assert_eq!(&frame[34..44], b"hello CCMP");
    }

    #[test]
    fn native_ccmp_rejects_bad_mic_and_missing_extended_iv() {
        let key = [0x55; 16];
        let mut frame = native_ccmp_qos_frame();
        protect_native_ccmp_frame(&key, &mut frame).unwrap();
        *frame.last_mut().unwrap() ^= 1;
        assert_eq!(
            unprotect_native_ccmp_frame(&key, &mut frame),
            Err(WifiError::CcmpAuthenticationFailed)
        );

        let mut malformed = native_ccmp_qos_frame();
        malformed[29] &= !0x20;
        assert_eq!(
            protect_native_ccmp_frame(&key, &mut malformed),
            Err(WifiError::MalformedCcmpFrame)
        );
    }

    #[test]
    fn scan_and_secure_association_are_deterministic() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.add_network(network()).unwrap();
        wifi.start(WifiMode::Station).unwrap();
        assert_eq!(wifi.scan(None).unwrap(), [network()]);
        assert_eq!(
            wifi.associate(AP, Some("short")),
            Err(WifiError::AuthenticationFailed)
        );
        wifi.associate(AP, Some("correct-horse")).unwrap();
        assert!(matches!(
            wifi.station_state(),
            WifiStationState::Associated { bssid: AP, .. }
        ));
        assert_eq!(wifi.channel(), 6);
    }

    #[test]
    fn destination_and_monitor_filters_cover_receive_paths() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.start(WifiMode::Station).unwrap();
        wifi.set_channel(6).unwrap();
        let other = MacAddress([0x02, 0, 0, 0, 0, 9]);
        let mut incoming = RadioFrame {
            protocol: RadioProtocol::Wifi,
            spectrum: wifi_spectrum(6),
            phy: "wifi-ht20".to_owned(),
            bytes: data_frame(other),
            mpdus: Vec::new(),
            origin: FrameOrigin::HostInjection,
        };
        assert!(!wifi.receive(&incoming).unwrap());
        incoming.bytes = data_frame(LOCAL);
        assert!(wifi.receive(&incoming).unwrap());
        assert_eq!(wifi.take_rx(), Some(data_frame(LOCAL)));

        wifi.start(WifiMode::Monitor).unwrap();
        incoming.bytes = data_frame(other);
        assert!(wifi.receive(&incoming).unwrap());
    }

    #[test]
    fn transmit_queue_carries_explicit_spectrum_and_origin() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.start(WifiMode::SoftAp).unwrap();
        wifi.set_channel(11).unwrap();
        assert_eq!(wifi.queue_tx(data_frame(MacAddress::BROADCAST)).unwrap(), 0);
        let (sequence, frame) = wifi.take_tx().unwrap();
        assert_eq!(sequence, 0);
        assert_eq!(frame.origin, FrameOrigin::Emulated);
        assert_eq!(frame.protocol, RadioProtocol::Wifi);
        assert_eq!(frame.spectrum, wifi_spectrum(11));
    }

    #[test]
    fn reset_zeroizes_volatile_state_but_keeps_scripted_peers() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.add_network(network()).unwrap();
        wifi.start(WifiMode::Station).unwrap();
        wifi.scan(None).unwrap();
        wifi.reset();
        assert_eq!(wifi.mode(), WifiMode::Disabled);
        wifi.start(WifiMode::Station).unwrap();
        assert_eq!(wifi.scan(None).unwrap().len(), 1);
    }

    #[test]
    fn soft_ap_beacons_clients_and_wifi6_capability_are_time_driven() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.configure_soft_ap(WifiSoftApConfiguration {
            ssid: "remu-ap".to_owned(),
            security: WifiSecurity::Wpa3Personal,
            passphrase: Some("correct-horse".to_owned()),
            max_clients: 1,
            beacon_interval: 100,
            wifi6: true,
        })
        .unwrap();
        wifi.start(WifiMode::SoftAp).unwrap();
        let client = wifi
            .associate_soft_ap_client(MacAddress([2, 0, 0, 0, 0, 9]))
            .unwrap();
        assert_ne!(client.aid, 0);
        assert_eq!(wifi.soft_ap_clients().count(), 1);
        assert_eq!(
            wifi.associate_soft_ap_client(MacAddress([2, 0, 0, 0, 0, 10])),
            Err(WifiError::SoftApFull)
        );

        wifi.advance_to(SimTime::ZERO);
        let (_, beacon) = wifi.take_tx().unwrap();
        assert_eq!(&beacon.bytes[..2], &[0x80, 0]);
        assert!(
            beacon
                .bytes
                .windows(4)
                .any(|element| element == [255, 2, 35, 1])
        );
        assert!(wifi.take_tx().is_none());
        wifi.advance_to(SimTime::from_ticks(102_400));
        assert!(wifi.take_tx().is_some());
    }

    #[test]
    fn esp_now_and_ack_retry_have_bounded_deterministic_queues() {
        let mut wifi = WifiEngine::new(LOCAL);
        wifi.start(WifiMode::Station).unwrap();
        let sequence = wifi.queue_esp_now(AP, b"hello").unwrap();
        let (first, frame) = wifi.take_tx().unwrap();
        assert_eq!(first, sequence);
        assert_eq!(&frame.bytes[24..30], &[127, 0x18, 0xfe, 0x34, 0, 0]);
        assert!(wifi.complete_tx(sequence, false).unwrap());
        assert_eq!(wifi.take_tx().unwrap().0, sequence);
        assert!(!wifi.complete_tx(sequence, true).unwrap());
        assert_eq!(
            wifi.queue_esp_now(AP, &[0; 251]),
            Err(WifiError::EspNowPayloadTooLong(251))
        );
    }
}
