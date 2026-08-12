use crate::{FrameOrigin, RadioFrame, RadioProtocol, Spectrum};
use aes::{
    Aes128,
    cipher::{BlockCipherEncrypt, KeyInit},
};
use ccm::{
    Ccm,
    aead::AeadInOut,
    consts::{U4, U13},
};
use remu_core::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;

const H4_COMMAND: u8 = 0x01;
const H4_ACL: u8 = 0x02;
const H4_EVENT: u8 = 0x04;
const EVT_DISCONNECTION_COMPLETE: u8 = 0x05;
const EVT_ENCRYPTION_CHANGE: u8 = 0x08;
const EVT_COMMAND_COMPLETE: u8 = 0x0e;
const EVT_COMMAND_STATUS: u8 = 0x0f;
const EVT_LE_META: u8 = 0x3e;
const STATUS_SUCCESS: u8 = 0x00;
const STATUS_UNKNOWN_COMMAND: u8 = 0x01;
const STATUS_UNKNOWN_CONNECTION: u8 = 0x02;
const STATUS_COMMAND_DISALLOWED: u8 = 0x0c;
const STATUS_INVALID_PARAMETERS: u8 = 0x12;

/// Six-byte Bluetooth device address in HCI wire order (least-significant byte first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BdAddress(pub [u8; 6]);

/// Stable connection handle allocated by the functional controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionHandle(pub u16);

/// BLE physical layer selected for a peer or connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlePhy {
    /// One-megabit uncoded PHY.
    #[default]
    Le1M,
    /// Two-megabit uncoded PHY.
    Le2M,
    /// Long-range coded PHY.
    LeCoded,
}

/// Deterministic remote BLE device visible to scan and connection procedures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlePeer {
    /// Public or random peer address.
    pub address: BdAddress,
    /// True when `address` is a random device address.
    pub random_address: bool,
    /// Advertising payload, limited to 31 bytes.
    pub advertising_data: Vec<u8>,
    /// Received signal strength in integer dBm.
    pub rssi_dbm: i8,
    /// Preferred connection PHY.
    pub phy: BlePhy,
}

/// Observable controller transition retained as deterministic evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum BleEvent {
    /// HCI command was decoded.
    Command {
        /// Bluetooth HCI command opcode.
        opcode: u16,
        /// Bluetooth status returned to the host.
        status: u8,
    },
    /// Advertising was enabled or disabled.
    Advertising {
        /// New advertising state.
        enabled: bool,
    },
    /// Scanning was enabled or disabled.
    Scanning {
        /// New scanning state.
        enabled: bool,
    },
    /// A connection was established.
    Connected {
        /// Newly allocated connection handle.
        handle: ConnectionHandle,
        /// Connected remote address.
        peer: BdAddress,
    },
    /// A connection was removed.
    Disconnected {
        /// Removed connection handle.
        handle: ConnectionHandle,
        /// HCI disconnection reason.
        reason: u8,
    },
    /// Link encryption state changed.
    Encryption {
        /// Affected connection handle.
        handle: ConnectionHandle,
        /// New encryption state.
        enabled: bool,
    },
    /// An ACL payload was queued for RF transmission.
    AclTx {
        /// Destination connection handle.
        handle: ConnectionHandle,
        /// ACL payload length in bytes.
        length: usize,
    },
}

/// BLE controller input error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BleError {
    /// H4 packet has an unsupported packet type.
    #[error("unsupported H4 packet type {0:#04x}")]
    UnsupportedPacketType(u8),
    /// Packet is truncated or has an inconsistent length.
    #[error("malformed HCI packet")]
    MalformedPacket,
    /// Advertising or scan-response data exceeds 31 bytes.
    #[error("BLE advertising data is {0} bytes; maximum is 31")]
    AdvertisingDataTooLong(usize),
    /// Scripted peer configuration is invalid.
    #[error("invalid BLE peer configuration")]
    InvalidPeer,
}

/// Direction bit carried in a Bluetooth LE data-channel CCM nonce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BleLinkDirection {
    /// Packet transmitted by the connection central.
    CentralToPeripheral,
    /// Packet transmitted by the connection peripheral.
    PeripheralToCentral,
}

/// Invalid native BLE encryption input or failed MIC verification.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BleLinkCryptoError {
    /// The PDU length byte does not describe the supplied bytes.
    #[error("malformed encrypted BLE data-channel PDU")]
    MalformedPdu,
    /// The 39-bit link-layer packet counter was exhausted.
    #[error("BLE encryption packet counter {0} exceeds 39 bits")]
    CounterOverflow(u64),
    /// AES-CCM authentication rejected the received payload.
    #[error("BLE data-channel MIC verification failed")]
    AuthenticationFailed,
}

fn ble_ccm_nonce(
    iv: &[u8; 8],
    counter: u64,
    direction: BleLinkDirection,
) -> Result<[u8; 13], BleLinkCryptoError> {
    if counter >= 1_u64 << 39 {
        return Err(BleLinkCryptoError::CounterOverflow(counter));
    }
    let mut nonce = [0_u8; 13];
    nonce[..5].copy_from_slice(&counter.to_le_bytes()[..5]);
    if direction == BleLinkDirection::PeripheralToCentral {
        nonce[4] |= 1 << 7;
    }
    nonce[5..].copy_from_slice(iv);
    Ok(nonce)
}

/// Encrypts one native BLE data-channel PDU and appends its four-byte MIC.
pub fn ble_link_encrypt_pdu(
    key: &[u8; 16],
    iv: &[u8; 8],
    counter: u64,
    direction: BleLinkDirection,
    pdu: &[u8],
) -> Result<Vec<u8>, BleLinkCryptoError> {
    let Some((&header, body)) = pdu.split_first() else {
        return Err(BleLinkCryptoError::MalformedPdu);
    };
    let Some((&length, payload)) = body.split_first() else {
        return Err(BleLinkCryptoError::MalformedPdu);
    };
    if usize::from(length) != payload.len() || payload.len() > usize::from(u8::MAX) - 4 {
        return Err(BleLinkCryptoError::MalformedPdu);
    }
    let nonce = ble_ccm_nonce(iv, counter, direction)?;
    let mut encrypted = payload.to_vec();
    let cipher = Ccm::<Aes128, U4, U13>::new(key.into());
    let mic = cipher
        .encrypt_inout_detached(
            (&nonce).into(),
            &[header & 0xe3],
            encrypted.as_mut_slice().into(),
        )
        .map_err(|_| BleLinkCryptoError::AuthenticationFailed)?;
    let mut result = Vec::with_capacity(pdu.len() + 4);
    result.push(header);
    result.push(length + 4);
    result.extend_from_slice(&encrypted);
    result.extend_from_slice(&mic);
    Ok(result)
}

/// Verifies and decrypts one native BLE data-channel PDU.
pub fn ble_link_decrypt_pdu(
    key: &[u8; 16],
    iv: &[u8; 8],
    counter: u64,
    direction: BleLinkDirection,
    pdu: &[u8],
) -> Result<Vec<u8>, BleLinkCryptoError> {
    let Some((&header, body)) = pdu.split_first() else {
        return Err(BleLinkCryptoError::MalformedPdu);
    };
    let Some((&length, payload_and_mic)) = body.split_first() else {
        return Err(BleLinkCryptoError::MalformedPdu);
    };
    if usize::from(length) != payload_and_mic.len() || payload_and_mic.len() < 4 {
        return Err(BleLinkCryptoError::MalformedPdu);
    }
    let payload_length = payload_and_mic.len() - 4;
    let (payload, mic) = payload_and_mic.split_at(payload_length);
    let nonce = ble_ccm_nonce(iv, counter, direction)?;
    let mut decrypted = payload.to_vec();
    let cipher = Ccm::<Aes128, U4, U13>::new(key.into());
    let mic = mic
        .try_into()
        .map_err(|_| BleLinkCryptoError::MalformedPdu)?;
    cipher
        .decrypt_inout_detached(
            (&nonce).into(),
            &[header & 0xe3],
            decrypted.as_mut_slice().into(),
            mic,
        )
        .map_err(|_| BleLinkCryptoError::AuthenticationFailed)?;
    let mut result = Vec::with_capacity(pdu.len() - 4);
    result.push(header);
    result.push((length - 4) as u8);
    result.extend_from_slice(&decrypted);
    Ok(result)
}

#[derive(Clone, Debug)]
struct Connection {
    peer: BdAddress,
    phy: BlePhy,
    encrypted: bool,
    key: Option<[u8; 16]>,
}

/// Deterministic Bluetooth LE controller exposing a standard H4 HCI transport.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct BleController {
    public_address: BdAddress,
    random_address: Option<BdAddress>,
    event_mask: u64,
    le_event_mask: u64,
    advertising_interval: u16,
    advertising_channels: u8,
    advertising_data: Vec<u8>,
    scan_response_data: Vec<u8>,
    advertising_enabled: bool,
    scan_interval: u16,
    scan_window: u16,
    scan_active: bool,
    scanning_enabled: bool,
    filter_duplicates: bool,
    address_resolution_enabled: bool,
    reported_peers: BTreeSet<BdAddress>,
    peers: BTreeMap<BdAddress, BlePeer>,
    connections: BTreeMap<ConnectionHandle, Connection>,
    next_handle: u16,
    default_phy: BlePhy,
    suggested_data_octets: u16,
    suggested_data_time: u16,
    hci_output: VecDeque<Vec<u8>>,
    rf_output: VecDeque<RadioFrame>,
    events: Vec<BleEvent>,
    random_state: u64,
    now: SimTime,
    next_advertising: Option<SimTime>,
    next_scan: Option<SimTime>,
}

impl BleController {
    /// Creates a reset controller with a stable public address and random seed.
    pub fn new(public_address: BdAddress, seed: u64) -> Self {
        Self {
            public_address,
            random_address: None,
            event_mask: u64::MAX,
            le_event_mask: u64::MAX,
            advertising_interval: 0x0800,
            advertising_channels: 0x07,
            advertising_data: Vec::new(),
            scan_response_data: Vec::new(),
            advertising_enabled: false,
            scan_interval: 0x0010,
            scan_window: 0x0010,
            scan_active: false,
            scanning_enabled: false,
            filter_duplicates: false,
            address_resolution_enabled: false,
            reported_peers: BTreeSet::new(),
            peers: BTreeMap::new(),
            connections: BTreeMap::new(),
            next_handle: 1,
            default_phy: BlePhy::Le1M,
            suggested_data_octets: 251,
            suggested_data_time: 2_120,
            hci_output: VecDeque::new(),
            rf_output: VecDeque::new(),
            events: Vec::new(),
            random_state: seed,
            now: SimTime::ZERO,
            next_advertising: None,
            next_scan: None,
        }
    }

    /// Adds or replaces a scripted remote peer.
    pub fn add_peer(&mut self, peer: BlePeer) -> Result<(), BleError> {
        if peer.advertising_data.len() > 31 {
            return Err(BleError::AdvertisingDataTooLong(
                peer.advertising_data.len(),
            ));
        }
        self.peers.insert(peer.address, peer);
        Ok(())
    }

    /// Removes all scripted remote peers.
    pub fn clear_peers(&mut self) {
        self.peers.clear();
        self.reported_peers.clear();
    }

    /// Consumes one complete H4 command or ACL packet.
    pub fn process_h4(&mut self, packet: &[u8]) -> Result<(), BleError> {
        let Some(packet_type) = packet.first().copied() else {
            return Err(BleError::MalformedPacket);
        };
        match packet_type {
            H4_COMMAND => self.process_command(packet),
            H4_ACL => self.process_acl(packet),
            other => Err(BleError::UnsupportedPacketType(other)),
        }
    }

    /// Removes the oldest complete H4 event or ACL packet for the host stack.
    pub fn take_h4_output(&mut self) -> Option<Vec<u8>> {
        self.hci_output.pop_front()
    }

    /// Returns whether an HCI event or ACL packet is waiting for the host stack.
    pub fn has_h4_output(&self) -> bool {
        !self.hci_output.is_empty()
    }

    /// Removes the oldest BLE link-layer PDU waiting for the shared medium.
    pub fn take_rf_output(&mut self) -> Option<RadioFrame> {
        self.rf_output.pop_front()
    }

    /// Advances periodic advertising and scan procedures to simulation time.
    ///
    /// Missed intervals are coalesced into one procedure at `now`; this keeps
    /// large instruction-time jumps bounded while preserving deterministic
    /// ordering and a stable next deadline.
    pub fn advance_to(&mut self, now: SimTime) {
        if now < self.now {
            return;
        }
        self.now = now;
        if self.advertising_enabled && self.next_advertising.is_none_or(|deadline| deadline <= now)
        {
            self.advertise_once();
            self.next_advertising = now
                .checked_add(remu_core::SimDuration::from_ticks(
                    u64::from(self.advertising_interval).max(1) * 625,
                ))
                .ok();
        }
        if self.scanning_enabled && self.next_scan.is_none_or(|deadline| deadline <= now) {
            self.scan_once();
            self.next_scan = now
                .checked_add(remu_core::SimDuration::from_ticks(
                    u64::from(self.scan_interval).max(1) * 625,
                ))
                .ok();
        }
    }

    /// Emits advertising PDUs on enabled primary channels for one deterministic interval.
    pub fn advertise_once(&mut self) {
        if !self.advertising_enabled {
            return;
        }
        let address = self.random_address.unwrap_or(self.public_address);
        for (bit, channel) in [(0x01, 37), (0x02, 38), (0x04, 39)] {
            if self.advertising_channels & bit == 0 {
                continue;
            }
            let mut pdu = Vec::with_capacity(8 + self.advertising_data.len());
            pdu.push(if self.random_address.is_some() {
                0x40
            } else {
                0x00
            });
            pdu.push(u8::try_from(6 + self.advertising_data.len()).expect("validated PDU length"));
            pdu.extend_from_slice(&address.0);
            pdu.extend_from_slice(&self.advertising_data);
            self.rf_output.push_back(RadioFrame {
                protocol: RadioProtocol::BluetoothLe,
                spectrum: ble_spectrum(channel),
                phy: "ble-1m".to_owned(),
                bytes: pdu,
                mpdus: Vec::new(),
                origin: FrameOrigin::Emulated,
            });
        }
    }

    /// Reports all visible scripted peers through standard LE advertising events.
    pub fn scan_once(&mut self) {
        if !self.scanning_enabled {
            return;
        }
        let peers: Vec<_> = self.peers.values().cloned().collect();
        for peer in peers {
            if self.filter_duplicates && self.reported_peers.contains(&peer.address) {
                continue;
            }
            self.reported_peers.insert(peer.address);
            self.push_advertising_report(&peer);
        }
    }

    /// Applies one BLE link-layer PDU received from the shared medium.
    pub fn receive_rf(&mut self, frame: &RadioFrame, rssi_dbm: i8) -> Result<bool, BleError> {
        if frame.protocol != RadioProtocol::BluetoothLe {
            return Ok(false);
        }
        if matches!(frame.spectrum.center_khz, 2_402_000 | 2_426_000 | 2_480_000) {
            if !self.scanning_enabled || frame.bytes.len() < 8 {
                return Ok(false);
            }
            let declared_length = usize::from(frame.bytes[1] & 0x3f);
            if declared_length < 6 || declared_length + 2 > frame.bytes.len() {
                return Err(BleError::MalformedPacket);
            }
            let address = BdAddress(
                frame.bytes[2..8]
                    .try_into()
                    .expect("checked advertising address"),
            );
            if self.filter_duplicates && self.reported_peers.contains(&address) {
                return Ok(false);
            }
            self.reported_peers.insert(address);
            self.push_advertising_report(&BlePeer {
                address,
                random_address: frame.bytes[0] & 0x40 != 0,
                advertising_data: frame.bytes[8..2 + declared_length].to_vec(),
                rssi_dbm,
                phy: BlePhy::Le1M,
            });
            return Ok(true);
        }
        if frame.bytes.len() < 4 {
            return Err(BleError::MalformedPacket);
        }
        let handle =
            ConnectionHandle(u16::from_le_bytes([frame.bytes[0], frame.bytes[1]]) & 0x0fff);
        let length = usize::from(u16::from_le_bytes([frame.bytes[2], frame.bytes[3]]));
        if frame.bytes.len() != length + 4 {
            return Err(BleError::MalformedPacket);
        }
        self.receive_acl(handle, &frame.bytes[4..])
    }

    /// Injects one valid incoming ACL payload for an established connection.
    pub fn receive_acl(
        &mut self,
        handle: ConnectionHandle,
        payload: &[u8],
    ) -> Result<bool, BleError> {
        if !self.connections.contains_key(&handle) || payload.len() > u16::MAX as usize {
            return Ok(false);
        }
        let raw_handle = handle.0 & 0x0fff;
        let mut packet = vec![H4_ACL];
        packet.extend_from_slice(&raw_handle.to_le_bytes());
        packet.extend_from_slice(
            &u16::try_from(payload.len())
                .expect("validated ACL payload length")
                .to_le_bytes(),
        );
        packet.extend_from_slice(payload);
        self.hci_output.push_back(packet);
        Ok(true)
    }

    /// Returns whether a connection exists and is encrypted.
    pub fn connection_encrypted(&self, handle: ConnectionHandle) -> Option<bool> {
        self.connections
            .get(&handle)
            .map(|connection| connection.encrypted)
    }

    /// Returns the negotiated PHY for an established connection.
    pub fn connection_phy(&self, handle: ConnectionHandle) -> Option<BlePhy> {
        self.connections
            .get(&handle)
            .map(|connection| connection.phy)
    }

    /// Returns the remote address for an established connection.
    pub fn connection_peer(&self, handle: ConnectionHandle) -> Option<BdAddress> {
        self.connections
            .get(&handle)
            .map(|connection| connection.peer)
    }

    /// Append-only controller evidence since the most recent reset.
    pub fn events(&self) -> &[BleEvent] {
        &self.events
    }

    #[allow(clippy::too_many_lines)]
    fn process_command(&mut self, packet: &[u8]) -> Result<(), BleError> {
        if packet.len() < 4 || packet.len() != usize::from(packet[3]) + 4 {
            return Err(BleError::MalformedPacket);
        }
        let opcode = u16::from_le_bytes([packet[1], packet[2]]);
        let params = &packet[4..];
        let status = match opcode {
            0x0c03 => {
                self.reset_volatile();
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x1001 => {
                self.command_complete(
                    opcode,
                    &[
                        STATUS_SUCCESS,
                        0x0c,
                        0x02,
                        0x00,
                        0x0c,
                        0x5d,
                        0x00,
                        0x01,
                        0x00,
                    ],
                );
                STATUS_SUCCESS
            }
            0x1002 => {
                let mut result = vec![STATUS_SUCCESS];
                result.extend_from_slice(&supported_commands());
                self.command_complete(opcode, &result);
                STATUS_SUCCESS
            }
            0x1003 => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 0, 0, 0, 0, 0x60, 0, 0, 0]);
                STATUS_SUCCESS
            }
            0x1005 if params.is_empty() => {
                self.command_complete(
                    opcode,
                    &[STATUS_SUCCESS, 0xfb, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00],
                );
                STATUS_SUCCESS
            }
            0x1009 => {
                let mut result = vec![STATUS_SUCCESS];
                result.extend_from_slice(&self.public_address.0);
                self.command_complete(opcode, &result);
                STATUS_SUCCESS
            }
            0x0c01 if params.len() == 8 => {
                self.event_mask = u64::from_le_bytes(params.try_into().expect("eight-byte mask"));
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x0c31 if params.len() == 1 && params[0] <= 1 => {
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x0c33 if params.len() == 7 => {
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x0c63 if params.len() == 8 => {
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x2001 if params.len() == 8 => {
                self.le_event_mask =
                    u64::from_le_bytes(params.try_into().expect("eight-byte mask"));
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x2002 if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 0xfb, 0x00, 0x08]);
                STATUS_SUCCESS
            }
            0x2003 if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 0xff, 0x49, 0x01, 0, 0, 0, 0, 0]);
                STATUS_SUCCESS
            }
            0x2005 if params.len() == 6 && valid_random_address(params) => {
                self.random_address = Some(BdAddress(params.try_into().expect("six-byte address")));
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x2006 if params.len() == 15 => self.set_advertising_parameters(opcode, params),
            0x2007 if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 0]);
                STATUS_SUCCESS
            }
            0x2008 if params.len() == 32 => self.set_advertising_data(opcode, params, false),
            0x2009 if params.len() == 32 => self.set_advertising_data(opcode, params, true),
            0x200a if params.len() == 1 => {
                let requested = params[0];
                let status = if requested <= 1 {
                    self.advertising_enabled = requested != 0;
                    self.next_advertising = (requested != 0).then_some(self.now);
                    self.events.push(BleEvent::Advertising {
                        enabled: requested != 0,
                    });
                    STATUS_SUCCESS
                } else {
                    STATUS_INVALID_PARAMETERS
                };
                self.command_complete(opcode, &[status]);
                status
            }
            0x200b if params.len() == 7 => self.set_scan_parameters(opcode, params),
            0x200c if params.len() == 2 => self.set_scan_enable(opcode, params),
            0x200d if params.len() == 25 => self.create_connection(opcode, params),
            0x200e if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_COMMAND_DISALLOWED]);
                STATUS_COMMAND_DISALLOWED
            }
            0x200f | 0x202a if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 8]);
                STATUS_SUCCESS
            }
            0x2017 if params.len() == 32 => {
                let cipher = Aes128::new_from_slice(&params[..16]).expect("AES-128 key length");
                let mut block: [u8; 16] = params[16..].try_into().expect("sixteen-byte plaintext");
                cipher.encrypt_block((&mut block).into());
                let mut result = vec![STATUS_SUCCESS];
                result.extend_from_slice(&block);
                self.command_complete(opcode, &result);
                STATUS_SUCCESS
            }
            0x2018 if params.is_empty() => {
                let value = self.next_random().to_le_bytes();
                let mut result = vec![STATUS_SUCCESS];
                result.extend_from_slice(&value);
                self.command_complete(opcode, &result);
                STATUS_SUCCESS
            }
            0x2019 if params.len() == 28 => self.start_encryption(opcode, params),
            0x201a if params.len() == 18 => self.long_term_key_reply(opcode, params, true),
            0x201b if params.len() == 2 => self.long_term_key_reply(opcode, params, false),
            0x201c if params.is_empty() => {
                self.command_complete(
                    opcode,
                    &[
                        STATUS_SUCCESS,
                        0xff,
                        0xff,
                        0xff,
                        0xff,
                        0x03,
                        0x00,
                        0x00,
                        0x00,
                    ],
                );
                STATUS_SUCCESS
            }
            0x2022 if params.len() == 6 => self.set_data_length(opcode, params),
            0x2023 if params.is_empty() => {
                let mut result = vec![STATUS_SUCCESS];
                result.extend_from_slice(&self.suggested_data_octets.to_le_bytes());
                result.extend_from_slice(&self.suggested_data_time.to_le_bytes());
                self.command_complete(opcode, &result);
                STATUS_SUCCESS
            }
            0x2024 if params.len() == 4 => {
                let octets = u16::from_le_bytes([params[0], params[1]]);
                let time = u16::from_le_bytes([params[2], params[3]]);
                let status = if !(27..=251).contains(&octets) || !(328..=2_120).contains(&time) {
                    STATUS_INVALID_PARAMETERS
                } else {
                    self.suggested_data_octets = octets;
                    self.suggested_data_time = time;
                    STATUS_SUCCESS
                };
                self.command_complete(opcode, &[status]);
                status
            }
            0x202d if params.len() == 1 && params[0] <= 1 => {
                self.address_resolution_enabled = params[0] != 0;
                self.command_complete(opcode, &[STATUS_SUCCESS]);
                STATUS_SUCCESS
            }
            0x202f if params.is_empty() => {
                self.command_complete(
                    opcode,
                    &[
                        STATUS_SUCCESS,
                        0xfb,
                        0x00,
                        0x48,
                        0x08,
                        0xfb,
                        0x00,
                        0x48,
                        0x08,
                    ],
                );
                STATUS_SUCCESS
            }
            0x2030 if params.len() == 2 => self.read_phy(opcode, params),
            0x2031 if params.len() == 3 => self.set_default_phy(opcode, params),
            0x2032 if params.len() == 7 => self.set_phy(opcode, params),
            0x203a if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 31, 0]);
                STATUS_SUCCESS
            }
            0x203b if params.is_empty() => {
                self.command_complete(opcode, &[STATUS_SUCCESS, 1]);
                STATUS_SUCCESS
            }
            0x1405 if params.len() == 2 => self.read_rssi(opcode, params),
            0x0406 if params.len() == 3 => self.disconnect(opcode, params),
            _ => {
                self.command_complete(opcode, &[STATUS_UNKNOWN_COMMAND]);
                STATUS_UNKNOWN_COMMAND
            }
        };
        self.events.push(BleEvent::Command { opcode, status });
        Ok(())
    }

    fn process_acl(&mut self, packet: &[u8]) -> Result<(), BleError> {
        if packet.len() < 5 {
            return Err(BleError::MalformedPacket);
        }
        let raw_handle = u16::from_le_bytes([packet[1], packet[2]]);
        let length = usize::from(u16::from_le_bytes([packet[3], packet[4]]));
        if packet.len() != length + 5 {
            return Err(BleError::MalformedPacket);
        }
        let handle = ConnectionHandle(raw_handle & 0x0fff);
        let Some(connection) = self.connections.get(&handle) else {
            return Ok(());
        };
        let mut pdu = Vec::with_capacity(length + 4);
        pdu.extend_from_slice(&raw_handle.to_le_bytes());
        pdu.extend_from_slice(
            &u16::try_from(length)
                .expect("HCI ACL length is encoded as u16")
                .to_le_bytes(),
        );
        pdu.extend_from_slice(&packet[5..]);
        self.rf_output.push_back(RadioFrame {
            protocol: RadioProtocol::BluetoothLe,
            spectrum: ble_spectrum(0),
            phy: match connection.phy {
                BlePhy::Le1M => "ble-1m",
                BlePhy::Le2M => "ble-2m",
                BlePhy::LeCoded => "ble-coded",
            }
            .to_owned(),
            bytes: pdu,
            mpdus: Vec::new(),
            origin: FrameOrigin::Emulated,
        });
        self.events.push(BleEvent::AclTx { handle, length });
        Ok(())
    }

    fn set_advertising_parameters(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let interval_min = u16::from_le_bytes([params[0], params[1]]);
        let interval_max = u16::from_le_bytes([params[2], params[3]]);
        let channels = params[13];
        let status = if interval_min < 0x0020
            || interval_max > 0x4000
            || interval_min > interval_max
            || channels == 0
            || channels & !0x07 != 0
        {
            STATUS_INVALID_PARAMETERS
        } else if self.advertising_enabled {
            STATUS_COMMAND_DISALLOWED
        } else {
            self.advertising_interval = interval_min;
            self.advertising_channels = channels;
            STATUS_SUCCESS
        };
        self.command_complete(opcode, &[status]);
        status
    }

    fn set_advertising_data(&mut self, opcode: u16, params: &[u8], scan_response: bool) -> u8 {
        let length = usize::from(params[0]);
        let status = if length > 31 {
            STATUS_INVALID_PARAMETERS
        } else {
            let destination = if scan_response {
                &mut self.scan_response_data
            } else {
                &mut self.advertising_data
            };
            destination.clear();
            destination.extend_from_slice(&params[1..=length]);
            STATUS_SUCCESS
        };
        self.command_complete(opcode, &[status]);
        status
    }

    fn set_scan_parameters(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let interval = u16::from_le_bytes([params[1], params[2]]);
        let window = u16::from_le_bytes([params[3], params[4]]);
        let status = if params[0] > 1
            || !(0x0004..=0x4000).contains(&interval)
            || window < 0x0004
            || window > interval
        {
            STATUS_INVALID_PARAMETERS
        } else if self.scanning_enabled {
            STATUS_COMMAND_DISALLOWED
        } else {
            self.scan_active = params[0] == 1;
            self.scan_interval = interval;
            self.scan_window = window;
            STATUS_SUCCESS
        };
        self.command_complete(opcode, &[status]);
        status
    }

    fn set_scan_enable(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let status = if params[0] > 1 || params[1] > 1 {
            STATUS_INVALID_PARAMETERS
        } else {
            self.scanning_enabled = params[0] == 1;
            self.next_scan = self.scanning_enabled.then_some(self.now);
            self.filter_duplicates = params[1] == 1;
            self.reported_peers.clear();
            self.events.push(BleEvent::Scanning {
                enabled: self.scanning_enabled,
            });
            STATUS_SUCCESS
        };
        self.command_complete(opcode, &[status]);
        status
    }

    fn create_connection(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let address = BdAddress(params[6..12].try_into().expect("six-byte peer address"));
        let status = if let Some(peer) = self.peers.get(&address).cloned() {
            let handle = ConnectionHandle(self.next_handle);
            self.next_handle = if self.next_handle == 0x0eff {
                1
            } else {
                self.next_handle + 1
            };
            self.connections.insert(
                handle,
                Connection {
                    peer: address,
                    phy: peer.phy,
                    encrypted: false,
                    key: None,
                },
            );
            self.command_status(opcode, STATUS_SUCCESS);
            self.push_connection_complete(handle, &peer);
            self.events.push(BleEvent::Connected {
                handle,
                peer: address,
            });
            return STATUS_SUCCESS;
        } else {
            STATUS_INVALID_PARAMETERS
        };
        self.command_status(opcode, status);
        status
    }

    fn disconnect(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let reason = params[2];
        let status = if self.connections.remove(&handle).is_some() {
            self.command_status(opcode, STATUS_SUCCESS);
            self.push_event(
                EVT_DISCONNECTION_COMPLETE,
                &[STATUS_SUCCESS, params[0], params[1], reason],
            );
            self.events.push(BleEvent::Disconnected { handle, reason });
            return STATUS_SUCCESS;
        } else {
            STATUS_UNKNOWN_CONNECTION
        };
        self.command_status(opcode, status);
        status
    }

    fn start_encryption(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let status = if let Some(connection) = self.connections.get_mut(&handle) {
            connection.key = Some(params[12..28].try_into().expect("sixteen-byte LTK"));
            connection.encrypted = true;
            self.command_status(opcode, STATUS_SUCCESS);
            self.push_event(
                EVT_ENCRYPTION_CHANGE,
                &[STATUS_SUCCESS, params[0], params[1], 1],
            );
            self.events.push(BleEvent::Encryption {
                handle,
                enabled: true,
            });
            return STATUS_SUCCESS;
        } else {
            STATUS_UNKNOWN_CONNECTION
        };
        self.command_status(opcode, status);
        status
    }

    fn long_term_key_reply(&mut self, opcode: u16, params: &[u8], accepted: bool) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let status = if let Some(connection) = self.connections.get_mut(&handle) {
            if accepted {
                connection.key = Some(params[2..18].try_into().expect("sixteen-byte LTK"));
                connection.encrypted = true;
            }
            STATUS_SUCCESS
        } else {
            STATUS_UNKNOWN_CONNECTION
        };
        let mut result = vec![status];
        result.extend_from_slice(&handle.0.to_le_bytes());
        self.command_complete(opcode, &result);
        status
    }

    fn set_data_length(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let octets = u16::from_le_bytes([params[2], params[3]]);
        let time = u16::from_le_bytes([params[4], params[5]]);
        let status = if !self.connections.contains_key(&handle) {
            STATUS_UNKNOWN_CONNECTION
        } else if !(27..=251).contains(&octets) || !(328..=2_120).contains(&time) {
            STATUS_INVALID_PARAMETERS
        } else {
            STATUS_SUCCESS
        };
        self.command_complete(opcode, &[status]);
        if status == STATUS_SUCCESS {
            let mut event = vec![0x07];
            event.extend_from_slice(&handle.0.to_le_bytes());
            event.extend_from_slice(&octets.to_le_bytes());
            event.extend_from_slice(&time.to_le_bytes());
            event.extend_from_slice(&octets.to_le_bytes());
            event.extend_from_slice(&time.to_le_bytes());
            self.push_event(EVT_LE_META, &event);
        }
        status
    }

    fn read_phy(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let Some(connection) = self.connections.get(&handle) else {
            self.command_complete(opcode, &[STATUS_UNKNOWN_CONNECTION, params[0], params[1]]);
            return STATUS_UNKNOWN_CONNECTION;
        };
        let phy = phy_code(connection.phy);
        self.command_complete(opcode, &[STATUS_SUCCESS, params[0], params[1], phy, phy]);
        STATUS_SUCCESS
    }

    fn set_default_phy(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let status = match preferred_phy(params[0], params[1], params[2]) {
            Some(phy) => {
                self.default_phy = phy;
                STATUS_SUCCESS
            }
            None => STATUS_INVALID_PARAMETERS,
        };
        self.command_complete(opcode, &[status]);
        status
    }

    fn set_phy(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let status = match preferred_phy(params[2], params[3], params[4]) {
            Some(phy) if self.connections.contains_key(&handle) => {
                self.connections
                    .get_mut(&handle)
                    .expect("checked handle")
                    .phy = phy;
                let mut event = vec![0x0c, STATUS_SUCCESS];
                event.extend_from_slice(&handle.0.to_le_bytes());
                event.extend_from_slice(&[phy_code(phy), phy_code(phy)]);
                self.push_event(EVT_LE_META, &event);
                STATUS_SUCCESS
            }
            Some(_) => STATUS_UNKNOWN_CONNECTION,
            None => STATUS_INVALID_PARAMETERS,
        };
        self.command_status(opcode, status);
        status
    }

    fn read_rssi(&mut self, opcode: u16, params: &[u8]) -> u8 {
        let handle = ConnectionHandle(u16::from_le_bytes([params[0], params[1]]) & 0x0fff);
        let status = if self.connections.contains_key(&handle) {
            STATUS_SUCCESS
        } else {
            STATUS_UNKNOWN_CONNECTION
        };
        self.command_complete(
            opcode,
            &[status, params[0], params[1], (-40_i8).cast_unsigned()],
        );
        status
    }

    fn push_advertising_report(&mut self, peer: &BlePeer) {
        if self.le_event_mask & (1 << 1) == 0 {
            return;
        }
        let mut params = vec![0x02, 1, 0x00, u8::from(peer.random_address)];
        params.extend_from_slice(&peer.address.0);
        params
            .push(u8::try_from(peer.advertising_data.len()).expect("validated advertising length"));
        params.extend_from_slice(&peer.advertising_data);
        params.push(peer.rssi_dbm.cast_unsigned());
        self.push_event(EVT_LE_META, &params);
    }

    fn push_connection_complete(&mut self, handle: ConnectionHandle, peer: &BlePeer) {
        if self.le_event_mask & 1 == 0 {
            return;
        }
        let mut params = vec![0x01, STATUS_SUCCESS];
        params.extend_from_slice(&handle.0.to_le_bytes());
        params.push(0x00);
        params.push(u8::from(peer.random_address));
        params.extend_from_slice(&peer.address.0);
        params.extend_from_slice(&0x0018_u16.to_le_bytes());
        params.extend_from_slice(&0_u16.to_le_bytes());
        params.extend_from_slice(&0x01f4_u16.to_le_bytes());
        params.push(0);
        self.push_event(EVT_LE_META, &params);
    }

    fn command_complete(&mut self, opcode: u16, return_parameters: &[u8]) {
        let mut params = vec![1];
        params.extend_from_slice(&opcode.to_le_bytes());
        params.extend_from_slice(return_parameters);
        self.push_event(EVT_COMMAND_COMPLETE, &params);
    }

    fn command_status(&mut self, opcode: u16, status: u8) {
        let mut params = vec![status, 1];
        params.extend_from_slice(&opcode.to_le_bytes());
        self.push_event(EVT_COMMAND_STATUS, &params);
    }

    fn push_event(&mut self, event_code: u8, params: &[u8]) {
        let mut event = vec![
            H4_EVENT,
            event_code,
            u8::try_from(params.len()).expect("HCI event parameters fit one byte"),
        ];
        event.extend_from_slice(params);
        self.hci_output.push_back(event);
    }

    fn reset_volatile(&mut self) {
        self.random_address = None;
        self.event_mask = u64::MAX;
        self.le_event_mask = u64::MAX;
        self.advertising_interval = 0x0800;
        self.advertising_channels = 0x07;
        self.advertising_data.clear();
        self.scan_response_data.clear();
        self.advertising_enabled = false;
        self.scan_interval = 0x0010;
        self.scan_window = 0x0010;
        self.scan_active = false;
        self.scanning_enabled = false;
        self.filter_duplicates = false;
        self.address_resolution_enabled = false;
        self.reported_peers.clear();
        self.connections.clear();
        self.next_handle = 1;
        self.default_phy = BlePhy::Le1M;
        self.suggested_data_octets = 251;
        self.suggested_data_time = 2_120;
        self.rf_output.clear();
        self.events.clear();
        self.next_advertising = None;
        self.next_scan = None;
    }

    fn next_random(&mut self) -> u64 {
        self.random_state ^= self.random_state << 13;
        self.random_state ^= self.random_state >> 7;
        self.random_state ^= self.random_state << 17;
        self.random_state
    }
}

fn valid_random_address(address: &[u8]) -> bool {
    address[5] & 0xc0 != 0
}

fn preferred_phy(all_phys: u8, tx_phys: u8, rx_phys: u8) -> Option<BlePhy> {
    if all_phys & !3 != 0 || tx_phys & !7 != 0 || rx_phys & !7 != 0 {
        return None;
    }
    let combined = match all_phys {
        0 => tx_phys & rx_phys,
        1 => rx_phys,
        2 => tx_phys,
        3 => 1,
        _ => return None,
    };
    if combined & 2 != 0 {
        Some(BlePhy::Le2M)
    } else if combined & 1 != 0 {
        Some(BlePhy::Le1M)
    } else if combined & 4 != 0 {
        Some(BlePhy::LeCoded)
    } else {
        None
    }
}

fn phy_code(phy: BlePhy) -> u8 {
    match phy {
        BlePhy::Le1M => 1,
        BlePhy::Le2M => 2,
        BlePhy::LeCoded => 3,
    }
}

fn ble_spectrum(channel: u8) -> Spectrum {
    let center_khz = match channel {
        37 => 2_402_000,
        38 => 2_426_000,
        39 => 2_480_000,
        data => 2_404_000 + u32::from(data) * 2_000,
    };
    Spectrum::new(center_khz, 2_000)
}

fn supported_commands() -> [u8; 64] {
    let mut commands = [0_u8; 64];
    commands[5] |= 1 << 6;
    commands[14] |= 1 << 7;
    commands[15] |= 1 << 1;
    commands[25] = 0xff;
    commands[26] = 0x1f;
    commands[27] = 0x7c;
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: BdAddress = BdAddress([1, 2, 3, 4, 5, 6]);
    const PEER: BdAddress = BdAddress([6, 5, 4, 3, 2, 1]);

    fn controller() -> BleController {
        let mut controller = BleController::new(LOCAL, 0x1234_5678_9abc_def0);
        controller
            .add_peer(BlePeer {
                address: PEER,
                random_address: false,
                advertising_data: vec![2, 1, 6],
                rssi_dbm: -42,
                phy: BlePhy::Le2M,
            })
            .unwrap();
        controller
    }

    #[test]
    fn reset_and_read_address_have_exact_h4_events() {
        let mut controller = controller();
        controller.process_h4(&[1, 0x03, 0x0c, 0]).unwrap();
        assert_eq!(
            controller.take_h4_output(),
            Some(vec![4, 0x0e, 4, 1, 3, 12, 0])
        );
        controller.process_h4(&[1, 0x09, 0x10, 0]).unwrap();
        assert_eq!(
            controller.take_h4_output(),
            Some(vec![4, 0x0e, 10, 1, 9, 16, 0, 1, 2, 3, 4, 5, 6])
        );
    }

    #[test]
    fn scan_reports_scripted_peer_and_filters_duplicates() {
        let mut controller = controller();
        controller.process_h4(&[1, 0x0c, 0x20, 2, 1, 1]).unwrap();
        assert_eq!(
            controller.take_h4_output().unwrap()[..7],
            [4, 0x0e, 4, 1, 0x0c, 0x20, 0]
        );
        controller.scan_once();
        let report = controller.take_h4_output().unwrap();
        assert_eq!(&report[..7], &[4, 0x3e, 15, 2, 1, 0, 0]);
        assert_eq!(&report[7..13], &PEER.0);
        controller.scan_once();
        assert!(controller.take_h4_output().is_none());
    }

    #[test]
    fn connection_acl_encryption_and_disconnect_cover_lifecycle() {
        let mut controller = controller();
        let mut create = vec![1, 0x0d, 0x20, 25];
        create.extend_from_slice(&0x0010_u16.to_le_bytes());
        create.extend_from_slice(&0x0010_u16.to_le_bytes());
        create.push(0);
        create.push(0);
        create.extend_from_slice(&PEER.0);
        create.push(0);
        create.extend_from_slice(&0x0018_u16.to_le_bytes());
        create.extend_from_slice(&0x0028_u16.to_le_bytes());
        create.extend_from_slice(&0_u16.to_le_bytes());
        create.extend_from_slice(&0x01f4_u16.to_le_bytes());
        create.extend_from_slice(&0_u16.to_le_bytes());
        create.extend_from_slice(&0_u16.to_le_bytes());
        assert_eq!(create.len(), 29);
        controller.process_h4(&create).unwrap();
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_COMMAND_STATUS);
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_LE_META);
        assert_eq!(
            controller.connection_phy(ConnectionHandle(1)),
            Some(BlePhy::Le2M)
        );

        controller.process_h4(&[2, 1, 0, 3, 0, 1, 2, 3]).unwrap();
        let rf = controller.take_rf_output().unwrap();
        assert_eq!(rf.protocol, RadioProtocol::BluetoothLe);
        assert_eq!(&rf.bytes[4..], &[1, 2, 3]);

        let mut encrypt = vec![1, 0x19, 0x20, 28, 1, 0];
        encrypt.extend_from_slice(&[0; 8]);
        encrypt.extend_from_slice(&[0; 2]);
        encrypt.extend_from_slice(&[0x5a; 16]);
        controller.process_h4(&encrypt).unwrap();
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_COMMAND_STATUS);
        assert_eq!(
            controller.take_h4_output().unwrap()[1],
            EVT_ENCRYPTION_CHANGE
        );
        assert_eq!(
            controller.connection_encrypted(ConnectionHandle(1)),
            Some(true)
        );

        controller.process_h4(&[1, 6, 4, 3, 1, 0, 0x13]).unwrap();
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_COMMAND_STATUS);
        assert_eq!(
            controller.take_h4_output().unwrap()[1],
            EVT_DISCONNECTION_COMPLETE
        );
        assert_eq!(controller.connection_encrypted(ConnectionHandle(1)), None);
    }

    #[test]
    fn advertising_uses_all_enabled_primary_channels() {
        let mut controller = controller();
        let mut data = vec![1, 8, 0x20, 32, 3, 2, 1, 6];
        data.resize(36, 0);
        controller.process_h4(&data).unwrap();
        controller.take_h4_output();
        controller.process_h4(&[1, 0x0a, 0x20, 1, 1]).unwrap();
        controller.take_h4_output();
        controller.advertise_once();
        let centers: Vec<_> = (0..3)
            .map(|_| controller.take_rf_output().unwrap().spectrum.center_khz)
            .collect();
        assert_eq!(centers, [2_402_000, 2_426_000, 2_480_000]);
    }

    #[test]
    fn simulation_time_drives_advertising_and_scanning_without_host_poll_hacks() {
        let mut controller = controller();
        controller.process_h4(&[1, 0x0a, 0x20, 1, 1]).unwrap();
        controller.take_h4_output();
        controller.process_h4(&[1, 0x0c, 0x20, 2, 1, 0]).unwrap();
        controller.take_h4_output();

        controller.advance_to(SimTime::ZERO);
        assert_eq!(controller.rf_output.len(), 3);
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_LE_META);
        controller.advance_to(SimTime::from_ticks(1));
        assert_eq!(controller.rf_output.len(), 3);
        assert!(controller.take_h4_output().is_none());
        controller.advance_to(SimTime::from_ticks(0x10 * 625));
        assert_eq!(controller.take_h4_output().unwrap()[1], EVT_LE_META);
    }

    #[test]
    fn common_le_host_initialization_commands_have_standard_success_shapes() {
        let mut controller = controller();
        for command in [
            vec![1, 0x05, 0x10, 0],
            vec![
                1, 0x01, 0x0c, 8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ],
            vec![1, 0x1c, 0x20, 0],
            vec![1, 0x2a, 0x20, 0],
            vec![1, 0x2f, 0x20, 0],
            vec![1, 0x3a, 0x20, 0],
            vec![1, 0x3b, 0x20, 0],
        ] {
            controller.process_h4(&command).unwrap();
            let response = controller.take_h4_output().unwrap();
            assert_eq!(response[1], EVT_COMMAND_COMPLETE);
            assert_eq!(response[6], STATUS_SUCCESS);
        }

        let mut encrypt = vec![1, 0x17, 0x20, 32];
        encrypt.extend_from_slice(&[0; 32]);
        controller.process_h4(&encrypt).unwrap();
        let response = controller.take_h4_output().unwrap();
        assert_eq!(
            &response[7..],
            &[
                0x66, 0xe9, 0x4b, 0xd4, 0xef, 0x8a, 0x2c, 0x3b, 0x88, 0x4c, 0xfa, 0x59, 0xca, 0x34,
                0x2b, 0x2e,
            ]
        );
    }

    #[test]
    fn native_data_channel_ccm_matches_firmware_derived_c6_transaction() {
        let key = [
            0xa2, 0xd7, 0x35, 0x99, 0x28, 0x97, 0x4f, 0xf1, 0x83, 0x60, 0x89, 0x9b, 0x80, 0x55,
            0x60, 0x83,
        ];
        let iv = [0x30, 0x31, 0x32, 0x33, 0xea, 0x47, 0xa4, 0x07];
        let transactions = [
            (
                &[0x0f, 1, 0x06][..],
                &[0x0f, 5, 0x5e, 0xde, 0x96, 0xdf, 0x76][..],
            ),
            (&[0x01, 0][..], &[0x01, 4, 0x42, 0x92, 0x45, 0xd7][..]),
            (
                &[0x0f, 2, 0x02, 0x13][..],
                &[0x0f, 6, 0x8f, 0x33, 0xa3, 0x5f, 0x87, 0xd4][..],
            ),
        ];
        for (counter, (plaintext, expected)) in transactions.into_iter().enumerate() {
            let encrypted = ble_link_encrypt_pdu(
                &key,
                &iv,
                counter as u64,
                BleLinkDirection::CentralToPeripheral,
                plaintext,
            )
            .unwrap();
            assert_eq!(encrypted, expected);
            assert_eq!(
                ble_link_decrypt_pdu(
                    &key,
                    &iv,
                    counter as u64,
                    BleLinkDirection::CentralToPeripheral,
                    &encrypted,
                )
                .unwrap(),
                plaintext
            );
        }
        let encrypted = transactions[0].1.to_vec();
        let mut corrupted = encrypted;
        *corrupted.last_mut().unwrap() ^= 1;
        assert_eq!(
            ble_link_decrypt_pdu(
                &key,
                &iv,
                0,
                BleLinkDirection::CentralToPeripheral,
                &corrupted,
            ),
            Err(BleLinkCryptoError::AuthenticationFailed)
        );
    }

    #[test]
    fn native_data_channel_ccm_matches_firmware_derived_s3_transaction() {
        let key = [
            0x54, 0x7f, 0x91, 0xc8, 0x36, 0xf0, 0x2c, 0x07, 0x6f, 0x02, 0x10, 0x4c, 0x8f, 0xaf,
            0xcc, 0x5b,
        ];
        let iv = [0x30, 0x31, 0x32, 0x33, 0x85, 0x4e, 0x5d, 0x41];
        let encrypted = ble_link_encrypt_pdu(
            &key,
            &iv,
            0,
            BleLinkDirection::CentralToPeripheral,
            &[0x0f, 1, 0x06],
        )
        .unwrap();
        assert_eq!(encrypted, [0x0f, 5, 0x95, 0xa5, 0x21, 0x6f, 0x92]);
        assert_eq!(
            ble_link_decrypt_pdu(
                &key,
                &iv,
                0,
                BleLinkDirection::CentralToPeripheral,
                &encrypted,
            )
            .unwrap(),
            [0x0f, 1, 0x06]
        );
        let encrypted_empty = ble_link_encrypt_pdu(
            &key,
            &iv,
            1,
            BleLinkDirection::CentralToPeripheral,
            &[0x05, 0],
        )
        .unwrap();
        assert_eq!(encrypted_empty, [0x05, 4, 0x1a, 0x20, 0x51, 0x1c]);
        let encrypted_terminate = ble_link_encrypt_pdu(
            &key,
            &iv,
            2,
            BleLinkDirection::CentralToPeripheral,
            &[0x0b, 2, 0x02, 0x13],
        )
        .unwrap();
        assert_eq!(
            encrypted_terminate,
            [0x0b, 6, 0xb3, 0x49, 0xc4, 0x27, 0xaf, 0x27]
        );
    }
}
