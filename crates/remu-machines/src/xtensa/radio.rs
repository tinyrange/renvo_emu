use super::{XtensaMachine, XtensaMachineError};
use remu_core::{AccessKind, AccessWidth, Bus, SimDuration};
use remu_radio::{
    BleController, BleLinkDirection, CoexistenceDecision, CoexistenceGrantId, CoexistenceRequest,
    DeliveryOutcome, FrameOrigin, MediumEvent, NodeId, RadioDmaDirection, RadioFrame,
    RadioLegalityRule, RadioPeer, RadioProtocol, RadioSubsystem, Receiver, ReplayArtifact,
    Spectrum, TransmissionId, TxRequest, WifiEngine, ble_link_decrypt_pdu, ble_link_encrypt_pdu,
};
const EMULATED_NODE: NodeId = NodeId(1);
const HOST_NODE: NodeId = NodeId(0);
// Native RWBLE interrupt causes, recovered from the revision-zero ROM ISR's
// register dispatch. These are hardware status bits, not symbol hooks: bit 5
// dispatches the programmed-slot END handler and bit 6 the SKIP handler. Bits
// 1 and 2 dispatch TX and RX respectively; bit 18 updates the RX-buffer ring.
const S3_RWBLE_SLEEP_WAKE_COMPLETE_INTERRUPT: u32 = 1;
const S3_RWBLE_SLEEP_WAKE_INTERRUPT: u32 = 1 << 3;
const S3_RWBLE_RX_INTERRUPT: u32 = 1 << 2;
const S3_RWBLE_TX_INTERRUPT: u32 = 1 << 1;
const S3_RWBLE_END_INTERRUPT: u32 = 1 << 5;
const S3_RWBLE_SKIP_INTERRUPT: u32 = 1 << 6;
const S3_BLE_INTERFRAME_SPACE_TICKS: u64 = 2_400;
const S3_BLE_1M_BYTE_TICKS: u64 = 8 * 16;
const S3_BLE_FINE_POSITION_TICKS: u64 = 8;
const S3_BLE_FINE_POSITIONS_PER_HALF_SLOT: u64 = 625;
const S3_BLE_HALF_SLOT_TICKS: u64 =
    S3_BLE_FINE_POSITION_TICKS * S3_BLE_FINE_POSITIONS_PER_HALF_SLOT;
const S3_BLE_COARSE_MASK: u64 = 0x0fff_ffff;
const S3_BLE_CLOCK_CYCLE_TICKS: u64 = (S3_BLE_COARSE_MASK + 1) * S3_BLE_HALF_SLOT_TICKS;
const BLE_ADVERTISING_ACCESS_ADDRESS: u32 = 0x8e89_bed6;

impl XtensaMachine {
    /// Attaches a deterministic external peer to the ESP32-S3 isolated RF
    /// medium. The peer observes emitted frames only and cannot access machine
    /// state.
    pub fn set_radio_peer(&mut self, peer: Box<dyn RadioPeer>) {
        self.radio_medium.set_peer(peer);
    }

    fn reset_coexistence(&mut self) -> Result<(), XtensaMachineError> {
        let active_airtime = self
            .radio_coexistence
            .owner()
            .is_some_and(|(_, end)| end > self.now);
        if active_airtime {
            if let Some((_, transmission)) = self.radio_coexistence_transmission.take() {
                self.radio_medium.truncate(transmission, self.now)?;
            }
        } else {
            self.radio_coexistence_transmission = None;
        }
        self.radio_coexistence.reset(self.now)?;
        Ok(())
    }

    fn power_down_coexistence(&mut self) -> Result<(), XtensaMachineError> {
        let Some((grant, _, end)) = self.radio_coexistence.active_grant() else {
            return Ok(());
        };
        if end <= self.now {
            return Ok(());
        }
        let prior = self.radio_coexistence_transmission.take();
        self.radio_legality.require(
            RadioSubsystem::Coexistence,
            RadioLegalityRule::CoexistenceOwnership,
            prior.is_some_and(|(mapped, _)| mapped == grant),
            self.now,
            format!("power-gated grant {grant:?} has no matching RF transmission"),
        )?;
        let (_, transmission) = prior.expect("validated coexistence transmission exists");
        self.radio_medium.truncate(transmission, self.now)?;
        self.radio_coexistence.power_down(self.now)?;
        Ok(())
    }

    fn apply_coexistence_preemption(
        &mut self,
        preempted: Option<CoexistenceGrantId>,
    ) -> Result<(), XtensaMachineError> {
        let Some(preempted) = preempted else {
            return Ok(());
        };
        let prior = self.radio_coexistence_transmission.take();
        self.radio_legality.require(
            RadioSubsystem::Coexistence,
            RadioLegalityRule::CoexistenceOwnership,
            prior.is_some_and(|(grant, _)| grant == preempted),
            self.now,
            format!("preempted grant {preempted:?} has no matching RF transmission"),
        )?;
        let (_, transmission) = prior.expect("validated coexistence transmission exists");
        self.radio_medium.truncate(transmission, self.now)?;
        Ok(())
    }

    fn record_coexistence_transmission(
        &mut self,
        grant: CoexistenceGrantId,
        transmission: TransmissionId,
    ) {
        self.radio_coexistence_transmission = Some((grant, transmission));
    }
}

#[derive(Clone)]
pub(super) struct PendingNativeBleTransmission {
    start: u64,
    slot_address: u32,
    event_index: u8,
    channel: u8,
    phy: &'static str,
    complete_event: bool,
    response_window: bool,
    tx_interrupt: bool,
    link_state: Option<u32>,
    remove_link_after_tx: bool,
    deferred_descriptor: Option<(u32, u32, u32)>,
    pdu: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct PendingNativeBleReception {
    start: u64,
    end: u64,
    slot_address: u32,
    event_index: u8,
    channel: u8,
    phy: &'static str,
    complete_on_receive: bool,
    link_state: Option<u32>,
    response: Option<PendingNativeBleTransmission>,
}

#[derive(Clone, Copy)]
struct S3BlePendingPhyUpdate {
    instant: u16,
    tx_phy: &'static str,
    rx_phy: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum S3BleEncryptionPhase {
    #[default]
    Unencrypted,
    EncReqReceived,
    SessionKeyReady,
    EncRspSent,
    StartReqSent,
    StartRspReceived,
    Encrypted,
}

pub(super) struct S3BleLinkSequence {
    expected_rx_sn: bool,
    event_counter: u16,
    active_event: u16,
    tx_phy: &'static str,
    rx_phy: &'static str,
    pending_phy_update: Option<S3BlePendingPhyUpdate>,
    encryption_phase: S3BleEncryptionPhase,
    encryption_skd: Option<[u8; 16]>,
    encryption_iv: Option<[u8; 8]>,
    session_key: Option<[u8; 16]>,
    rx_packet_counter: u64,
    tx_packet_counter: u64,
    last_tx_plaintext: Option<Vec<u8>>,
    last_tx_wire: Option<Vec<u8>>,
}

impl Default for S3BleLinkSequence {
    fn default() -> Self {
        Self {
            expected_rx_sn: false,
            event_counter: 0,
            active_event: 0,
            tx_phy: "ble-1m",
            rx_phy: "ble-1m",
            pending_phy_update: None,
            encryption_phase: S3BleEncryptionPhase::Unencrypted,
            encryption_skd: None,
            encryption_iv: None,
            session_key: None,
            rx_packet_counter: 0,
            tx_packet_counter: 0,
            last_tx_plaintext: None,
            last_tx_wire: None,
        }
    }
}

impl S3BleLinkSequence {
    pub(super) fn begin_event(&mut self) -> Result<(&'static str, &'static str), String> {
        self.active_event = self.event_counter;
        if let Some(update) = self.pending_phy_update {
            if update.instant == self.active_event {
                self.tx_phy = update.tx_phy;
                self.rx_phy = update.rx_phy;
                self.pending_phy_update = None;
            } else if self.active_event.wrapping_sub(update.instant) < 0x8000 {
                return Err(format!(
                    "BLE PHY update instant {} passed at connection event {}",
                    update.instant, self.active_event
                ));
            }
        }
        self.event_counter = self.event_counter.wrapping_add(1);
        Ok((self.tx_phy, self.rx_phy))
    }

    fn encryption_material(&self) -> Result<([u8; 16], [u8; 8]), String> {
        let key = self
            .session_key
            .ok_or_else(|| "S3 BLE encryption is active without a session key".to_owned())?;
        let iv = self
            .encryption_iv
            .ok_or_else(|| "S3 BLE encryption is active without a negotiated IV".to_owned())?;
        Ok((key, iv))
    }

    fn rx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            S3BleEncryptionPhase::StartReqSent
                | S3BleEncryptionPhase::StartRspReceived
                | S3BleEncryptionPhase::Encrypted
        )
    }

    fn tx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            S3BleEncryptionPhase::StartRspReceived | S3BleEncryptionPhase::Encrypted
        )
    }

    pub(super) fn decode_received(&self, received: &[u8]) -> Result<Vec<u8>, String> {
        if !self.rx_encryption_active() {
            return Ok(received.to_vec());
        }
        let header = received
            .first()
            .copied()
            .ok_or_else(|| "encrypted S3 BLE PDU has no header".to_owned())?;
        let received_new = (header & 0x08 != 0) == self.expected_rx_sn;
        let counter = if received_new {
            self.rx_packet_counter
        } else {
            self.rx_packet_counter.checked_sub(1).ok_or_else(|| {
                "encrypted S3 BLE retransmission precedes packet counter zero".to_owned()
            })?
        };
        let (key, iv) = self.encryption_material()?;
        ble_link_decrypt_pdu(
            &key,
            &iv,
            counter,
            BleLinkDirection::CentralToPeripheral,
            received,
        )
        .map_err(|error| format!("encrypted S3 BLE RX counter {counter}: {error}"))
    }

    pub(super) fn observe_security_ecb(
        &mut self,
        key: [u8; 16],
        input: [u8; 16],
        output: [u8; 16],
    ) -> Result<bool, String> {
        let Some(skd) = self.encryption_skd else {
            return Ok(false);
        };
        if input[..8] != skd[..8] {
            return Ok(false);
        }
        if self.encryption_phase != S3BleEncryptionPhase::EncReqReceived
            || self.session_key.is_some()
        {
            return Err(format!(
                "S3 BLE session-key ECB completed during {:?}",
                self.encryption_phase
            ));
        }
        // The LTK is intentionally not fixed by the emulator. Its exact value
        // comes from the guest host stack, while SKD correlation binds this
        // native transaction to one live link.
        let _firmware_ltk = key;
        self.encryption_skd = Some(input);
        self.session_key = Some(output);
        self.encryption_phase = S3BleEncryptionPhase::SessionKeyReady;
        Ok(true)
    }

    pub(super) fn prepare_transmitted(&mut self, pdu: &[u8]) -> Result<Vec<u8>, String> {
        if self.tx_encryption_active() {
            if let (Some(plaintext), Some(wire)) =
                (self.last_tx_plaintext.as_ref(), self.last_tx_wire.as_ref())
            {
                if plaintext == pdu {
                    return Ok(wire.clone());
                }
                return Err("S3 BLE encrypted TX changed before acknowledgement".to_owned());
            }
            self.observe_transmitted_control(pdu)?;
            let (key, iv) = self.encryption_material()?;
            let wire = ble_link_encrypt_pdu(
                &key,
                &iv,
                self.tx_packet_counter,
                BleLinkDirection::PeripheralToCentral,
                pdu,
            )
            .map_err(|error| {
                format!(
                    "native S3 BLE TX CCM counter {} failed: {error}",
                    self.tx_packet_counter
                )
            })?;
            self.last_tx_plaintext = Some(pdu.to_vec());
            self.last_tx_wire = Some(wire.clone());
            Ok(wire)
        } else {
            self.observe_transmitted_control(pdu)?;
            Ok(pdu.to_vec())
        }
    }

    pub(super) fn acknowledge_encrypted_transmission(
        &mut self,
        peer_nesn: bool,
    ) -> Result<(), String> {
        let acknowledged = self
            .last_tx_plaintext
            .as_ref()
            .and_then(|pdu| pdu.first())
            .is_some_and(|header| peer_nesn != (header & 0x08 != 0));
        if acknowledged {
            self.tx_packet_counter = self
                .tx_packet_counter
                .checked_add(1)
                .ok_or_else(|| "S3 BLE TX packet counter overflow".to_owned())?;
            self.last_tx_plaintext = None;
            self.last_tx_wire = None;
        }
        Ok(())
    }

    fn observe_transmitted_control(&mut self, pdu: &[u8]) -> Result<(), String> {
        if pdu.first().is_none_or(|header| header & 3 != 3) || pdu.len() < 3 {
            return Ok(());
        }
        match pdu[2] {
            0x04 => {
                if pdu.len() != 15 || self.encryption_phase != S3BleEncryptionPhase::SessionKeyReady
                {
                    return Err(format!(
                        "S3 LL_ENC_RSP emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                let skd = self
                    .encryption_skd
                    .ok_or_else(|| "S3 LL_ENC_RSP has no negotiated SKD".to_owned())?;
                if pdu[3..11] != skd[8..] {
                    return Err("S3 LL_ENC_RSP SKDs differs from AES input".to_owned());
                }
                let iv = self
                    .encryption_iv
                    .as_mut()
                    .ok_or_else(|| "S3 LL_ENC_RSP has no IV storage".to_owned())?;
                iv[4..].copy_from_slice(&pdu[11..15]);
                self.encryption_phase = S3BleEncryptionPhase::EncRspSent;
            }
            0x05 => {
                if pdu.len() != 3 || self.encryption_phase != S3BleEncryptionPhase::EncRspSent {
                    return Err(format!(
                        "S3 LL_START_ENC_REQ emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = S3BleEncryptionPhase::StartReqSent;
            }
            0x06 => {
                // The S3 controller retains four zero work bytes in the
                // firmware-owned descriptor. The canonical Core form and
                // that exact recovered descriptor form encode the same
                // parameterless LL control response.
                let firmware_length =
                    pdu.len() == 3 || (pdu.len() == 7 && pdu[3..] == [0, 0, 0, 0]);
                if !firmware_length
                    || self.encryption_phase != S3BleEncryptionPhase::StartRspReceived
                {
                    return Err(format!(
                        "S3 LL_START_ENC_RSP length {} bytes {pdu:02x?} emitted during {:?}",
                        pdu.len(),
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = S3BleEncryptionPhase::Encrypted;
            }
            0x03 => return Err("S3 peripheral emitted central-role LL_ENC_REQ".to_owned()),
            _ => {}
        }
        Ok(())
    }

    pub(super) fn observe_received(&mut self, received: &[u8]) -> Result<bool, String> {
        let Some(header) = received.first().copied() else {
            return Ok(false);
        };
        let received_sn = header & (1 << 3) != 0;
        if received_sn != self.expected_rx_sn {
            return Ok(false);
        }
        self.expected_rx_sn = !self.expected_rx_sn;
        let received_encrypted = self.rx_encryption_active();
        let terminates_link = header & 3 == 3
            && received.get(1).copied() == Some(2)
            && received.get(2).copied() == Some(0x02);
        if header & 3 == 3 && received.len() >= 3 {
            match received[2] {
                0x03 => {
                    if received.len() != 25
                        || self.encryption_phase != S3BleEncryptionPhase::Unencrypted
                    {
                        return Err(format!(
                            "S3 LL_ENC_REQ length {} received during {:?}",
                            received.len(),
                            self.encryption_phase
                        ));
                    }
                    let mut skd = [0_u8; 16];
                    skd[..8].copy_from_slice(&received[13..21]);
                    let mut iv = [0_u8; 8];
                    iv[..4].copy_from_slice(&received[21..25]);
                    self.encryption_skd = Some(skd);
                    self.encryption_iv = Some(iv);
                    self.rx_packet_counter = 0;
                    self.tx_packet_counter = 0;
                    self.encryption_phase = S3BleEncryptionPhase::EncReqReceived;
                }
                0x06 => {
                    if received.len() != 3
                        || self.encryption_phase != S3BleEncryptionPhase::StartReqSent
                    {
                        return Err(format!(
                            "S3 LL_START_ENC_RSP received during {:?}",
                            self.encryption_phase
                        ));
                    }
                    self.encryption_phase = S3BleEncryptionPhase::StartRspReceived;
                }
                0x04 | 0x05 => {
                    return Err(format!(
                        "S3 central sent peripheral-role encryption opcode {:#04x}",
                        received[2]
                    ));
                }
                _ => {}
            }
        }
        if received_encrypted {
            self.rx_packet_counter = self
                .rx_packet_counter
                .checked_add(1)
                .ok_or_else(|| "S3 BLE RX packet counter overflow".to_owned())?;
        }
        if header & 3 != 3
            || received.len() < 7
            || received.get(1).copied() != Some(5)
            || received.get(2).copied() != Some(0x18)
        {
            return Ok(terminates_link);
        }
        let select_phy = |requested, current| match requested {
            0 => Some(current),
            1 => Some("ble-1m"),
            2 => Some("ble-2m"),
            3 => Some("ble-coded"),
            _ => None,
        };
        let central_tx = received[3];
        let central_rx = received[4];
        let Some(rx_phy) = select_phy(central_tx, self.rx_phy) else {
            return Err(format!("invalid central TX PHY value {central_tx}"));
        };
        let Some(tx_phy) = select_phy(central_rx, self.tx_phy) else {
            return Err(format!("invalid central RX PHY value {central_rx}"));
        };
        let instant = u16::from_le_bytes([received[5], received[6]]);
        let instant_delta = instant.wrapping_sub(self.active_event);
        if !(6..0x8000).contains(&instant_delta) {
            return Err(format!(
                "BLE PHY update instant {instant} is {instant_delta} events after current event {}",
                self.active_event
            ));
        }
        if self.pending_phy_update.is_some() {
            return Err("overlapping BLE PHY update procedures".to_owned());
        }
        self.pending_phy_update = Some(S3BlePendingPhyUpdate {
            instant,
            tx_phy,
            rx_phy,
        });
        Ok(false)
    }
}

impl XtensaMachine {
    /// Returns the S3 functional Wi-Fi engine when its clock/reset domain is ready.
    pub fn wifi_engine(&mut self) -> Result<&mut WifiEngine, XtensaMachineError> {
        if !self.syscon.wifi_ready() {
            return Err(XtensaMachineError::RadioNotReady("Wi-Fi"));
        }
        Ok(&mut self.radio_wifi)
    }

    /// Returns the S3 functional BLE HCI controller when its domain is ready.
    pub fn ble_controller(&mut self) -> Result<&mut BleController, XtensaMachineError> {
        if !self.syscon.ble_ready() {
            return Err(XtensaMachineError::RadioNotReady("Bluetooth LE"));
        }
        Ok(&mut self.radio_ble)
    }

    /// Injects one explicit packet into the host-isolated deterministic medium.
    pub fn inject_radio_frame(
        &mut self,
        protocol: RadioProtocol,
        spectrum: Spectrum,
        phy: impl Into<String>,
        bytes: Vec<u8>,
        power_dbm: i16,
    ) -> Result<(), XtensaMachineError> {
        self.inject_radio_frame_at(self.now, protocol, spectrum, phy, bytes, power_dbm)
    }

    /// Schedules one explicit host packet at a simulation timestamp.
    pub fn inject_radio_frame_at(
        &mut self,
        at: remu_core::SimTime,
        protocol: RadioProtocol,
        spectrum: Spectrum,
        phy: impl Into<String>,
        bytes: Vec<u8>,
        power_dbm: i16,
    ) -> Result<(), XtensaMachineError> {
        self.radio_medium.tune_receiver(Receiver {
            node: EMULATED_NODE,
            protocol,
            spectrum,
            sensitivity_dbm: -100,
        })?;
        let duration = frame_duration(bytes.len());
        self.radio_medium.transmit(TxRequest {
            source: HOST_NODE,
            start: at,
            end: at
                .checked_add(duration)
                .map_err(|_| XtensaMachineError::TimeOverflow)?,
            power_dbm,
            frame: RadioFrame {
                protocol,
                spectrum,
                phy: phy.into(),
                bytes,
                origin: FrameOrigin::HostInjection,
            },
        })?;
        Ok(())
    }

    /// Returns a versioned snapshot of S3 RF and coexistence events.
    pub fn radio_replay_artifact(&self) -> ReplayArtifact {
        ReplayArtifact::new(
            self.radio_medium.profile().clone(),
            self.radio_medium.events().to_vec(),
        )
        .with_coexistence_events(self.radio_coexistence.events().to_vec())
    }

    pub(super) fn service_radio(&mut self) -> Result<u64, XtensaMachineError> {
        let wifi_ready = self.syscon.wifi_ready();
        let ble_ready = self.syscon.ble_ready();
        let coexistence_ready =
            self.syscon.radio_clock_enable() != 0 && self.syscon.radio_reset_enable() == 0;
        let reset_generation = self.syscon.radio_reset_generation();
        if reset_generation != self.radio_reset_generation {
            self.reset_coexistence()?;
            self.pending_native_wifi.clear();
            self.pending_native_ble_transmissions.clear();
            self.pending_native_ble_receptions.clear();
            self.pending_native_ble_slot_completions.clear();
            self.native_ble_link_sequences.clear();
            self.radio_reset_generation = reset_generation;
        }
        if self
            .radio_coexistence
            .active_grant()
            .is_some_and(|(_, protocol, _)| {
                !coexistence_ready
                    || match protocol {
                        RadioProtocol::Wifi => !wifi_ready,
                        RadioProtocol::BluetoothLe => !ble_ready,
                        RadioProtocol::Ieee802154 => true,
                    }
            })
        {
            self.power_down_coexistence()?;
        }
        self.radio_legality.observe_domain(
            RadioSubsystem::Wifi,
            wifi_ready,
            Some(reset_generation),
            self.now,
        )?;
        self.radio_legality.observe_domain(
            RadioSubsystem::BluetoothLe,
            ble_ready,
            Some(reset_generation),
            self.now,
        )?;
        self.radio_legality.observe_domain(
            RadioSubsystem::Coexistence,
            coexistence_ready,
            Some(reset_generation),
            self.now,
        )?;
        self.radio_medium.advance_to(self.now)?;
        self.radio_coexistence.advance_to(self.now)?;
        self.complete_native_ble_slot_states()?;
        self.ble_exchange_memory.advance_to(self.now);
        let mut events = 0_u64;
        if self.ble_lp_clock.take_wake_request() {
            // During modem sleep the genuine ROM leaves only RWBLE cause bit 3
            // enabled. The LP sequencer asserts that cause after consuming the
            // wake-request strobe, which enters r_rwip_wakeup through the real
            // controller ISR.
            self.ble_exchange_memory
                .raise_interrupt(S3_RWBLE_SLEEP_WAKE_INTERRUPT);
            events = events.saturating_add(1);
        }
        if self.ble_lp_clock.take_wake_completion_request(self.now) {
            // r_rwip_wakeup finishes PHY/clock restoration, sets LP control
            // bit 3, and enables RWBLE cause bit 0. Hardware answers that
            // second-stage command with cause 0, dispatching the genuine ROM
            // wake-end path before normal scheduling resumes.
            self.ble_exchange_memory
                .raise_interrupt(S3_RWBLE_SLEEP_WAKE_COMPLETE_INTERRUPT);
            events = events.saturating_add(1);
        }
        if self.syscon.ble_ready() {
            self.radio_ble.advance_to(self.now);
        }
        if self.syscon.wifi_ready() {
            self.radio_wifi.advance_to(self.now);
        }
        events = events.saturating_add(self.complete_radio_receptions()?);
        events = events.saturating_add(self.complete_native_wifi_transmissions()?);
        events = events.saturating_add(self.service_native_ble_crypt()?);
        events = events.saturating_add(self.submit_native_wifi_frames()?);
        events = events.saturating_add(self.submit_native_ble_frames()?);
        events = events.saturating_add(self.service_pending_native_ble_frames()?);
        events = events.saturating_add(self.submit_protocol_frames()?);
        let wifi_pending = self.radio_wifi.has_rx() || self.wifi_mac.interrupt_pending();
        let ble_pending = self.radio_ble.has_h4_output();
        let rwble_pending = self.ble_exchange_memory.interrupt_pending();
        self.radio_legality
            .observe_interrupt(RadioSubsystem::Wifi, wifi_pending, self.now)?;
        self.radio_legality.observe_interrupt(
            RadioSubsystem::BluetoothLe,
            ble_pending || rwble_pending,
            self.now,
        )?;
        self.update_matrix_source(0, wifi_pending)?;
        self.update_matrix_source(4, ble_pending)?;
        self.update_matrix_source(8, rwble_pending)?;
        Ok(events)
    }

    fn submit_native_wifi_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut submitted = 0_u64;
        while let Some(descriptor) = self.wifi_mac.take_tx_descriptor() {
            self.radio_legality.validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                descriptor.address,
                4,
                8,
                8,
                self.now,
            )?;
            let control = self.bus.read(
                u64::from(descriptor.address),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let buffer = self.bus.read(
                u64::from(descriptor.address.wrapping_add(4)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let capacity = (control as usize) & 0x0fff;
            let wire_length = ((control as usize) >> 12) & 0x0fff;
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                remu_radio::RadioLegalityRule::DmaLength,
                wire_length > 4,
                self.now,
                format!(
                    "TX DMA wire length {wire_length} does not contain a MAC frame and 4-byte FCS"
                ),
            )?;
            // The S3 MAC length includes a hardware-generated four-byte FCS,
            // while descriptor capacity covers only guest-owned frame bytes.
            // Genuine authentication traffic uses exactly capacity+4.
            let length = wire_length - 4;
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                remu_radio::RadioLegalityRule::DmaLength,
                length <= capacity,
                self.now,
                format!("TX DMA MAC-frame length {length} exceeds descriptor capacity {capacity}"),
            )?;
            self.radio_legality.validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                buffer as u32,
                2,
                length,
                4095,
                self.now,
            )?;
            let bytes = (0..length)
                .map(|offset| {
                    self.bus
                        .read(
                            u64::from((buffer as u32).wrapping_add(offset as u32)),
                            AccessWidth::Byte,
                            AccessKind::Read,
                            self.now,
                        )
                        .map(|value| value as u8)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let duration = frame_duration(bytes.len());
            let end = self
                .now
                .checked_add(duration)
                .map_err(|_| XtensaMachineError::TimeOverflow)?;
            let pending = crate::native_wifi::PendingNativeWifiTransmission::new(
                descriptor.queue,
                &bytes,
                end,
                self.wifi_mac.tx_ack_timeout(descriptor.queue),
            )
            .ok_or(XtensaMachineError::TimeOverflow)?;
            let decision = self.radio_coexistence.request(CoexistenceRequest {
                protocol: RadioProtocol::Wifi,
                start: self.now,
                duration,
                priority: 8,
                preemptible: true,
            })?;
            let CoexistenceDecision::Granted {
                id: grant,
                protocol: granted_protocol,
                preempted,
                ..
            } = decision
            else {
                self.radio_legality.require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CompletionWithoutOperation,
                    self.wifi_mac.complete_tx(
                        descriptor.queue,
                        remu_devices::EspWifiTxOutcome::TransmitError,
                    ),
                    self.now,
                    format!(
                        "native TX queue {} rejected its RF-arbitration failure completion",
                        descriptor.queue
                    ),
                )?;
                continue;
            };
            self.apply_coexistence_preemption(preempted)?;
            self.radio_legality.validate_coexistence_ownership(
                RadioSubsystem::Wifi,
                RadioProtocol::Wifi,
                granted_protocol,
                self.now,
            )?;
            let transmission = self.radio_medium.transmit(TxRequest {
                source: EMULATED_NODE,
                start: self.now,
                end,
                power_dbm: 0,
                frame: RadioFrame {
                    protocol: RadioProtocol::Wifi,
                    spectrum: Spectrum::new(2_412_000, 20_000),
                    phy: "wifi-ht20".to_owned(),
                    bytes,
                    origin: FrameOrigin::Emulated,
                },
            })?;
            if pending.ack_receiver.is_some() {
                self.radio_medium.tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::Wifi,
                    spectrum: Spectrum::new(2_412_000, 20_000),
                    sensitivity_dbm: -100,
                })?;
            }
            self.pending_native_wifi.push(pending);
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    fn complete_native_wifi_transmissions(&mut self) -> Result<u64, XtensaMachineError> {
        let due = self
            .pending_native_wifi
            .iter()
            .copied()
            .filter(|pending| pending.deadline <= self.now)
            .collect::<Vec<_>>();
        for pending in &due {
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::CompletionWithoutOperation,
                self.wifi_mac.tx_active(pending.queue),
                self.now,
                format!(
                    "native TX queue {} reached its completion deadline without an active hardware operation",
                    pending.queue
                ),
            )?;
            let outcome = if pending.ack_receiver.is_some() {
                remu_devices::EspWifiTxOutcome::AckTimeout
            } else {
                remu_devices::EspWifiTxOutcome::Success
            };
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::CompletionWithoutOperation,
                self.wifi_mac.complete_tx(pending.queue, outcome),
                self.now,
                format!("native TX queue {} rejected its completion", pending.queue),
            )?;
        }
        self.pending_native_wifi
            .retain(|pending| pending.deadline > self.now);
        Ok(due.len() as u64)
    }

    fn complete_radio_receptions(&mut self) -> Result<u64, XtensaMachineError> {
        let new_events = &self.radio_medium.events()[self.radio_event_cursor..];
        let mut deliveries = Vec::new();
        for event in new_events {
            let MediumEvent::Reception {
                id,
                receiver: EMULATED_NODE,
                outcome: DeliveryOutcome::Delivered,
            } = event
            else {
                continue;
            };
            if let Some((frame, signal_dbm, received_at)) = self
                .radio_medium
                .events()
                .iter()
                .find_map(|candidate| match candidate {
                    MediumEvent::Submitted {
                        id: candidate_id,
                        request,
                    } if candidate_id == id => Some((
                        request.frame.clone(),
                        self.radio_medium.received_power_dbm(
                            request.source,
                            EMULATED_NODE,
                            request.power_dbm,
                        ),
                        request.start,
                    )),
                    _ => None,
                })
            {
                deliveries.push((frame, signal_dbm, received_at));
            }
        }
        self.radio_event_cursor = self.radio_medium.events().len();
        let mut completed = 0_u64;
        for (frame, signal_dbm, received_at) in deliveries {
            match frame.protocol {
                RadioProtocol::Wifi => {
                    if let Some(index) = self
                        .pending_native_wifi
                        .iter()
                        .position(|pending| pending.accepts_ack(&frame.bytes, received_at))
                    {
                        let pending = self.pending_native_wifi.remove(index);
                        self.radio_legality.require(
                            RadioSubsystem::Wifi,
                            RadioLegalityRule::CompletionWithoutOperation,
                            self.wifi_mac.tx_active(pending.queue),
                            self.now,
                            format!("ACK arrived for inactive native TX queue {}", pending.queue),
                        )?;
                        self.radio_legality.require(
                            RadioSubsystem::Wifi,
                            RadioLegalityRule::CompletionWithoutOperation,
                            self.wifi_mac.complete_tx(
                                pending.queue,
                                remu_devices::EspWifiTxOutcome::Success,
                            ),
                            self.now,
                            format!(
                                "native TX queue {} rejected its ACK completion",
                                pending.queue
                            ),
                        )?;
                        completed = completed.saturating_add(1);
                        continue;
                    }
                    let wifi_mac = self.wifi_mac.clone();
                    let native = self.write_native_wifi_rx(&wifi_mac, &frame.bytes);
                    if self.syscon.wifi_ready() && self.radio_wifi.receive(&frame).unwrap_or(false)
                    {
                        completed = completed.saturating_add(1);
                    } else if native {
                        completed = completed.saturating_add(1);
                    }
                }
                RadioProtocol::BluetoothLe if self.syscon.ble_ready() => {
                    let native = self.write_native_ble_rx(&frame, signal_dbm, received_at)?;
                    if self
                        .radio_ble
                        .receive_rf(
                            &frame,
                            signal_dbm.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8,
                        )
                        .unwrap_or(false)
                    {
                        completed = completed.saturating_add(1);
                    } else if native {
                        completed = completed.saturating_add(1);
                    }
                }
                RadioProtocol::BluetoothLe | RadioProtocol::Ieee802154 => {}
            }
        }
        Ok(completed)
    }

    fn write_native_ble_rx(
        &mut self,
        frame: &RadioFrame,
        signal_dbm: i16,
        received_at: remu_core::SimTime,
    ) -> Result<bool, XtensaMachineError> {
        self.pending_native_ble_receptions
            .retain(|pending| pending.end >= self.now.ticks());
        let Some((activity_index, activity)) = self
            .pending_native_ble_receptions
            .iter()
            .enumerate()
            .find(|pending| {
                pending.1.start <= self.now.ticks()
                    && pending.1.end >= self.now.ticks()
                    && frame.spectrum.overlaps(s3_ble_spectrum(pending.1.channel))
                    && pending.1.phy == frame.phy
            })
            .map(|(index, pending)| (index, pending.clone()))
        else {
            return Ok(false);
        };
        if frame.bytes.len() < 2 {
            return Ok(false);
        }

        let current = self.ble_exchange_memory.rx_buffer_current() & 0x7fff;
        let Some(descriptor) = self.ble_exchange_memory.resolve_em_address(current) else {
            return Ok(false);
        };
        let Some(status) = self.read_native_ble_u16(descriptor.wrapping_add(2)) else {
            return Ok(false);
        };
        if status & 0x8000 == 0 {
            return Ok(false);
        }
        let Some(buffer_offset) = self.read_native_ble_u16(descriptor.wrapping_add(18)) else {
            return Ok(false);
        };
        let Some(buffer) = self.ble_exchange_memory.resolve_em_address(buffer_offset) else {
            return Ok(false);
        };
        let Some(next) = self.read_native_ble_u16(descriptor) else {
            return Ok(false);
        };
        let received = if let Some(link_state) = activity.link_state {
            let decoded = self
                .native_ble_link_sequences
                .entry(link_state)
                .or_default()
                .decode_received(&frame.bytes);
            let (plaintext, encryption_error) = match decoded {
                Ok(plaintext) => (Some(plaintext), None),
                Err(detail) => (None, Some(detail)),
            };
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                encryption_error.is_none(),
                self.now,
                encryption_error.unwrap_or_default(),
            )?;
            plaintext.expect("legality check established BLE plaintext")
        } else {
            frame.bytes.clone()
        };
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::SchedulerState,
            received.len() >= 2,
            self.now,
            "native BLE decryption produced a truncated PDU",
        )?;
        let mut link_terminated = false;
        if let Some(link_state) = activity.link_state {
            let observed = self
                .native_ble_link_sequences
                .entry(link_state)
                .or_default()
                .observe_received(&received);
            let (terminated, sequence_error) = match observed {
                Ok(terminated) => (terminated, None),
                Err(detail) => (false, Some(detail)),
            };
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                sequence_error.is_none(),
                self.now,
                sequence_error.unwrap_or_default(),
            )?;
            link_terminated = terminated;
        }
        let header = u16::from_le_bytes([received[0], received[1]]);
        let coarse = ((received_at.ticks() / S3_BLE_HALF_SLOT_TICKS) & S3_BLE_COARSE_MASK) as u32;
        let fine = (S3_BLE_FINE_POSITIONS_PER_HALF_SLOT
            - 1
            - (received_at.ticks() % S3_BLE_HALF_SLOT_TICKS) / S3_BLE_FINE_POSITION_TICKS)
            as u16;
        // The revision-zero ROM's CONNECT_IND path reconstructs the 28-bit
        // receive clock from +8/+10 and consumes the low ten bits at +12 as
        // the descending fine position. Word +14 is RXCHASS (receive/privacy
        // status), not the RF channel; a nonzero value there deliberately
        // enters the resolving-list validation path.
        let raw_rssi = signal_dbm.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8 as u8;
        let metadata = [
            u16::from(raw_rssi),
            (coarse & 0xffff) as u16,
            (coarse >> 16) as u16,
            fine | (u16::from(activity.event_index & 0x1f) << 11),
            0,
            0,
        ];
        if !self.write_native_ble_bytes(buffer, &received[2..])
            || !self.write_native_ble_u16(descriptor, next | 0x8000)
            || !self.write_native_ble_u16(descriptor.wrapping_add(4), header)
            || metadata.iter().enumerate().any(|(index, value)| {
                !self.write_native_ble_u16(descriptor.wrapping_add(6 + (index as u32 * 2)), *value)
            })
            || !self.write_native_ble_u16(descriptor.wrapping_add(2), status & !0x8000)
        {
            return Ok(false);
        }
        self.ble_exchange_memory.advance_rx_buffer(next & 0x7fff);
        let response_scheduled = activity.response.is_some();
        if let Some(mut response) = activity.response {
            self.pending_native_ble_receptions.remove(activity_index);
            self.pending_native_ble_slot_completions
                .retain(|(due, slot, state)| {
                    *due != activity.end || *slot != activity.slot_address || *state != 4
                });
            let canceled = self.ble_exchange_memory.cancel_radio_completion(
                remu_core::SimTime::from_ticks(activity.end),
                S3_RWBLE_END_INTERRUPT,
            );
            if !canceled {
                return Ok(false);
            }
            if self
                .set_native_ble_slot_state(activity.slot_address, 2)
                .is_err()
            {
                return Ok(false);
            }
            if let Some((cs_address, current_descriptor, access_address)) =
                response.deferred_descriptor
            {
                let Some(current_header) =
                    self.read_native_ble_u16(current_descriptor.wrapping_add(2))
                else {
                    return Ok(false);
                };
                let peer_nesn = received[0] & 0x04 != 0;
                let peer_sn = received[0] & 0x08 != 0;
                let transmitted_sn = current_header & 0x08 != 0;
                let acknowledged = peer_nesn != transmitted_sn;
                let link_state = activity
                    .link_state
                    .expect("connection response has stable native link identity");
                let acknowledgement = self
                    .native_ble_link_sequences
                    .entry(link_state)
                    .or_default()
                    .acknowledge_encrypted_transmission(peer_nesn);
                let acknowledgement_error = acknowledgement.err();
                self.radio_legality.require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::SchedulerState,
                    acknowledgement_error.is_none(),
                    self.now,
                    acknowledgement_error.unwrap_or_default(),
                )?;
                let (descriptor, transmitted_sn) = if acknowledged {
                    let Some(linkage) = self.read_native_ble_u16(current_descriptor) else {
                        return Ok(false);
                    };
                    let next_offset = linkage & 0x7fff;
                    if next_offset == 0 {
                        return Ok(false);
                    }
                    let Some(next_descriptor) =
                        self.ble_exchange_memory.resolve_em_address(next_offset)
                    else {
                        return Ok(false);
                    };
                    if next_descriptor == current_descriptor {
                        return Ok(false);
                    }
                    let Some(next_linkage) = self.read_native_ble_u16(next_descriptor) else {
                        return Ok(false);
                    };
                    // Bit 15 is the exchange-memory ownership handoff. Return
                    // the acknowledged record to firmware and claim the next
                    // queued record before raising RX, matching the state the
                    // ROM observes in its link-layer ISR.
                    if !self.write_native_ble_u16(current_descriptor, linkage | 0x8000)
                        || !self.write_native_ble_u16(next_descriptor, next_linkage & 0x7fff)
                    {
                        return Ok(false);
                    }
                    if !self.write_native_ble_u16(cs_address.wrapping_add(28), next_offset) {
                        return Ok(false);
                    }
                    (None, !transmitted_sn)
                } else {
                    (Some(current_descriptor), transmitted_sn)
                };
                let sequence_bits = (u16::from(!peer_sn) << 2) | (u16::from(transmitted_sn) << 3);
                if let Some(descriptor) = descriptor {
                    let Some(mut response_header) =
                        self.read_native_ble_u16(descriptor.wrapping_add(2))
                    else {
                        return Ok(false);
                    };
                    // RWBLE owns the data-channel sequence bits. NESN in the
                    // response acknowledges the accepted peer SN.
                    response_header = (response_header & !0x000c) | sequence_bits;
                    if response_header >> 8 == 0 && response_header & 0x0003 == 0 {
                        // An empty ring record is transmitted as the
                        // data-channel empty PDU (LLID=1); LLID=0 is never
                        // legal on air.
                        response_header |= 1;
                    }
                    if !self.write_native_ble_u16(descriptor.wrapping_add(2), response_header) {
                        return Ok(false);
                    }
                    response.deferred_descriptor = Some((cs_address, descriptor, access_address));
                } else {
                    // Ownership advances immediately, but the newly exposed
                    // ring entry is for a future connection event. Hardware
                    // acknowledges the peer in this event with an empty PDU.
                    response.pdu = vec![(1 | sequence_bits) as u8, 0];
                    response.deferred_descriptor = None;
                }
            }
            self.ble_exchange_memory
                .raise_interrupt(S3_RWBLE_RX_INTERRUPT);
            response.start = self
                .now
                .checked_add(SimDuration::from_ticks(S3_BLE_INTERFRAME_SPACE_TICKS))
                .map(|time| time.ticks())
                .unwrap_or(u64::MAX);
            response.remove_link_after_tx = link_terminated;
            let insertion = self
                .pending_native_ble_transmissions
                .iter()
                .position(|pending| pending.start > response.start)
                .unwrap_or(self.pending_native_ble_transmissions.len());
            self.pending_native_ble_transmissions
                .insert(insertion, response);
        } else if activity.complete_on_receive {
            self.pending_native_ble_receptions.remove(activity_index);
            self.pending_native_ble_slot_completions
                .retain(|(due, slot, state)| {
                    *due != activity.end || *slot != activity.slot_address || *state != 4
                });
            let canceled = self.ble_exchange_memory.cancel_radio_completion(
                remu_core::SimTime::from_ticks(activity.end),
                S3_RWBLE_END_INTERRUPT,
            );
            if !canceled {
                return Ok(false);
            }
            if self
                .set_native_ble_slot_state(activity.slot_address, 4)
                .is_err()
            {
                return Ok(false);
            }
            self.ble_exchange_memory
                .raise_interrupt(S3_RWBLE_RX_INTERRUPT | S3_RWBLE_END_INTERRUPT);
        } else {
            if self
                .set_native_ble_slot_state(activity.slot_address, 2)
                .is_err()
            {
                return Ok(false);
            }
            self.ble_exchange_memory
                .raise_interrupt(S3_RWBLE_RX_INTERRUPT);
        }
        if link_terminated && !response_scheduled {
            if let Some(link_state) = activity.link_state {
                self.native_ble_link_sequences.remove(&link_state);
            }
        }
        Ok(true)
    }

    fn write_native_ble_u16(&mut self, address: u32, value: u16) -> bool {
        self.bus
            .write(
                u64::from(address),
                AccessWidth::HalfWord,
                u64::from(value),
                self.now,
            )
            .is_ok()
    }

    fn write_native_ble_bytes(&mut self, address: u32, bytes: &[u8]) -> bool {
        bytes.iter().enumerate().all(|(offset, byte)| {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .is_ok()
        })
    }

    fn write_native_wifi_rx(
        &mut self,
        wifi_mac: &remu_devices::Esp32S3WifiMacHandle,
        frame: &[u8],
    ) -> bool {
        let Some(descriptor) = wifi_mac.rx_descriptor() else {
            return false;
        };
        let Ok(control) = self.bus.read(
            u64::from(descriptor.address),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        if control as u32 & (1 << 31) == 0 {
            return false;
        }
        let Ok(buffer) = self.bus.read(
            u64::from(descriptor.address.wrapping_add(4)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        let Ok(next) = self.bus.read(
            u64::from(descriptor.address.wrapping_add(8)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        let capacity = (control as usize) & 0x0fff;
        let rx_match = wifi_mac.rx_match_mask(frame.get(4..10).unwrap_or_default());
        if rx_match == 0 {
            return false;
        }
        let metadata = s3_wifi_rx_metadata(frame, rx_match, self.now);
        let total = metadata.len().saturating_add(frame.len()).saturating_add(4);
        if buffer == 0 || total > capacity || total > 0x0fff {
            return false;
        }
        if !self.write_native_wifi_bytes(buffer as u32, &metadata)
            || !self
                .write_native_wifi_bytes((buffer as u32).wrapping_add(metadata.len() as u32), frame)
            || !self.write_native_wifi_bytes(
                (buffer as u32).wrapping_add((metadata.len() + frame.len()) as u32),
                &[0; 4],
            )
            || self
                .bus
                .write(
                    u64::from(descriptor.address),
                    AccessWidth::Word,
                    u64::from((control as u32 & 0x0000_0fff) | ((total as u32) << 12) | (1 << 30)),
                    self.now,
                )
                .is_err()
        {
            return false;
        }
        wifi_mac.complete_rx_descriptor(descriptor.address, next as u32);
        true
    }

    fn write_native_wifi_bytes(&mut self, address: u32, bytes: &[u8]) -> bool {
        bytes.iter().enumerate().all(|(offset, byte)| {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .is_ok()
        })
    }

    fn submit_protocol_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut frames = Vec::new();
        if self.syscon.wifi_ready() {
            while let Some((_, frame)) = self.radio_wifi.take_tx() {
                frames.push((frame, 8));
            }
        }
        if self.syscon.ble_ready() {
            while let Some(frame) = self.radio_ble.take_rf_output() {
                frames.push((frame, 9));
            }
        }
        let mut submitted = 0_u64;
        for (frame, priority) in frames {
            let duration = frame_duration(frame.bytes.len());
            let decision = self.radio_coexistence.request(CoexistenceRequest {
                protocol: frame.protocol,
                start: self.now,
                duration,
                priority,
                preemptible: true,
            })?;
            let CoexistenceDecision::Granted {
                id: grant,
                protocol: granted_protocol,
                preempted,
                ..
            } = decision
            else {
                continue;
            };
            self.apply_coexistence_preemption(preempted)?;
            self.radio_legality.validate_coexistence_ownership(
                match frame.protocol {
                    RadioProtocol::Wifi => RadioSubsystem::Wifi,
                    RadioProtocol::BluetoothLe => RadioSubsystem::BluetoothLe,
                    RadioProtocol::Ieee802154 => RadioSubsystem::Ieee802154,
                },
                frame.protocol,
                granted_protocol,
                self.now,
            )?;
            let transmission = self.radio_medium.transmit(TxRequest {
                source: EMULATED_NODE,
                start: self.now,
                end: self
                    .now
                    .checked_add(duration)
                    .map_err(|_| XtensaMachineError::TimeOverflow)?,
                power_dbm: 0,
                frame,
            })?;
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }
}

fn frame_duration(length: usize) -> SimDuration {
    SimDuration::from_ticks(
        u64::try_from(length.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(8),
    )
}

fn s3_ble_spectrum(channel: u8) -> Spectrum {
    let center_khz = match channel {
        37 => 2_402_000,
        38 => 2_426_000,
        39 => 2_480_000,
        0..=10 => 2_404_000 + u32::from(channel) * 2_000,
        11..=36 => 2_428_000 + u32::from(channel - 11) * 2_000,
        _ => 2_402_000,
    };
    Spectrum::new(center_khz, 2_000)
}

fn replace_s3_ble_aux_pointer(frame: &mut [u8], channel_control: u8, offset_phy: u16) -> bool {
    if frame.len() < 4 || frame[0] & 0x0f != 7 {
        return false;
    }
    let extended_header_length = usize::from(frame[2] & 0x3f);
    let Some(extended_header_end) = 3_usize.checked_add(extended_header_length) else {
        return false;
    };
    if extended_header_length == 0 || extended_header_end > frame.len() {
        return false;
    }
    let flags = frame[3];
    if flags & (1 << 4) == 0 {
        return false;
    }
    let mut cursor = 4_usize;
    for (bit, length) in [(0, 6_usize), (1, 6), (2, 1), (3, 2)] {
        if flags & (1 << bit) != 0 {
            let Some(next) = cursor.checked_add(length) else {
                return false;
            };
            cursor = next;
        }
    }
    if cursor
        .checked_add(3)
        .is_none_or(|end| end > extended_header_end)
    {
        return false;
    }
    frame[cursor] = channel_control;
    frame[cursor + 1..cursor + 3].copy_from_slice(&offset_phy.to_le_bytes());
    true
}

// The native control structure retains the controller event identity in the
// low five bits of CS+2. Scheduler table slots are recycled independently, so
// their rolling slot number cannot be used in RX descriptor metadata.
fn s3_ble_event_index(control: u16) -> u8 {
    (control & 0x1f) as u8
}

fn s3_wifi_rx_metadata(frame: &[u8], rx_match: u8, at: remu_core::SimTime) -> [u8; 48] {
    let mut metadata = [0_u8; 48];
    metadata[0] = (-40_i8) as u8;
    metadata[3] = (rx_match & 0x0f) << 4;
    metadata[8] = 1 << 1;
    metadata[11] = (u8::from(frame.get(4).is_some_and(|address| address & 1 != 0)) << 7) | 1;
    metadata[12..16].copy_from_slice(&(at.ticks() as u32).to_le_bytes());
    metadata[20] = (-95_i8) as u8;
    let signal_length = frame.len().saturating_add(4).min(0x0fff) as u32;
    metadata[44..48].copy_from_slice(&signal_length.to_le_bytes());
    metadata
}

include!("radio_ble.rs");

#[cfg(test)]
#[path = "radio_tests.rs"]
mod tests;
