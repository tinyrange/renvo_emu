use super::{MachineError, RiscVMachine};
use crate::TargetId;
use remu_core::{AccessKind, AccessWidth, Bus, SimDuration};
use remu_devices::{
    EspC6BleBasebandHandle, EspC6BleControlHandle, EspIeee802154Command, EspIeee802154Handle,
};
use remu_radio::{
    BleController, BleLinkDirection, CoexistenceDecision, CoexistenceGrantId, CoexistenceRequest,
    DeliveryOutcome, ExtendedAddress, FrameOrigin, Ieee802154CcaMode, Ieee802154Error,
    Ieee802154Mac, Ieee802154RxOutcome, MediumEvent, NodeId, PanInterface, RadioActivity,
    RadioDmaDirection, RadioFrame, RadioLegalityRule, RadioProtocol, RadioSubsystem, Receiver,
    ReplayArtifact, ShortAddress, Spectrum, TransmissionId, TxRequest, WifiEngine,
    ble_link_decrypt_pdu, ble_link_encrypt_pdu,
};

const EMULATED_NODE: NodeId = NodeId(1);
const HOST_NODE: NodeId = NodeId(0);
const C6_BLE_INTERFRAME_SPACE_TICKS: u64 = 2_400;

#[derive(Clone)]
pub(super) struct PendingNativeBleReception {
    start: remu_core::SimTime,
    end: remu_core::SimTime,
    schedule_address: u32,
    state: u32,
    spectrum: Spectrum,
    phy: &'static str,
    rx_buffer_identifier: u16,
}

pub(super) struct PendingNativeBleTransmission {
    start: remu_core::SimTime,
    spectrum: Spectrum,
    phy: &'static str,
    bytes: Vec<u8>,
    response: Option<PendingNativeBleReception>,
}

#[derive(Clone, Copy)]
struct C6BlePendingPhyUpdate {
    instant: u16,
    tx_phy: &'static str,
    rx_phy: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum C6BleEncryptionPhase {
    #[default]
    Unencrypted,
    EncReqReceived,
    EncRspSent,
    SessionKeyReady,
    StartReqSent,
    StartRspReceived,
    Encrypted,
}

pub(super) struct C6BleLinkSequence {
    pub(super) expected_rx_sn: bool,
    pub(super) tx_sn: bool,
    pub(super) awaiting_tx_ack: bool,
    pub(super) last_tx: Option<Vec<u8>>,
    event_counter: u16,
    active_event: u16,
    tx_phy: &'static str,
    rx_phy: &'static str,
    pending_phy_update: Option<C6BlePendingPhyUpdate>,
    encryption_phase: C6BleEncryptionPhase,
    encryption_skd: Option<[u8; 16]>,
    encryption_iv: Option<[u8; 8]>,
    session_key: Option<[u8; 16]>,
    rx_packet_counter: u64,
    tx_packet_counter: u64,
    last_tx_encrypted: bool,
    pending_native_rx_counter: Option<u64>,
}

impl Default for C6BleLinkSequence {
    fn default() -> Self {
        Self {
            expected_rx_sn: false,
            tx_sn: false,
            awaiting_tx_ack: false,
            last_tx: None,
            event_counter: 0,
            active_event: 0,
            tx_phy: "ble-1m",
            rx_phy: "ble-1m",
            pending_phy_update: None,
            encryption_phase: C6BleEncryptionPhase::Unencrypted,
            encryption_skd: None,
            encryption_iv: None,
            session_key: None,
            rx_packet_counter: 0,
            tx_packet_counter: 0,
            last_tx_encrypted: false,
            pending_native_rx_counter: None,
        }
    }
}

impl C6BleLinkSequence {
    pub(super) fn begin_event(&mut self) -> Result<&'static str, String> {
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
        Ok(self.rx_phy)
    }

    pub(super) fn tx_phy(&self) -> &'static str {
        self.tx_phy
    }

    pub(super) fn expects_central_response(&self) -> bool {
        self.encryption_phase == C6BleEncryptionPhase::StartReqSent
    }

    pub(super) fn hardware_filters_empty(&self, received: &[u8]) -> Result<bool, String> {
        if self.encryption_phase == C6BleEncryptionPhase::SessionKeyReady {
            return Ok(received.get(1).copied() == Some(0));
        }
        if self.rx_encryption_active() {
            return self
                .decode_received(received)
                .map(|plaintext| plaintext.get(1).copied() == Some(0));
        }
        Ok(false)
    }

    pub(super) fn native_rx_dma_frame(&self, received: &[u8]) -> Result<Vec<u8>, String> {
        if self.encryption_phase == C6BleEncryptionPhase::Encrypted {
            self.decode_received(received)
        } else {
            Ok(received.to_vec())
        }
    }

    pub(super) fn allows_silent_event_end(&self) -> bool {
        self.encryption_phase == C6BleEncryptionPhase::StartRspReceived
    }

    pub(super) fn complete_hardware_filtered_rx(&mut self) {
        // Authenticated empty PDUs update the baseband's ACK and packet-counter
        // state but never reach the firmware RX ring or its explicit CCM path.
        self.pending_native_rx_counter = None;
    }

    fn encryption_material(&self) -> Result<([u8; 16], [u8; 8]), String> {
        let Some(key) = self.session_key else {
            return Err(
                "BLE encryption is active without a firmware-derived session key".to_owned(),
            );
        };
        let Some(iv) = self.encryption_iv else {
            return Err("BLE encryption is active without a negotiated IV".to_owned());
        };
        Ok((key, iv))
    }

    fn rx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            C6BleEncryptionPhase::StartReqSent
                | C6BleEncryptionPhase::StartRspReceived
                | C6BleEncryptionPhase::Encrypted
        )
    }

    fn tx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            C6BleEncryptionPhase::StartRspReceived | C6BleEncryptionPhase::Encrypted
        )
    }

    pub(super) fn decode_received(&self, received: &[u8]) -> Result<Vec<u8>, String> {
        if !self.rx_encryption_active() {
            return Ok(received.to_vec());
        }
        let Some(header) = received.first().copied() else {
            return Err("encrypted BLE connection PDU has no header".to_owned());
        };
        let received_new = (header & (1 << 3) != 0) == self.expected_rx_sn;
        let counter = if received_new {
            self.rx_packet_counter
        } else {
            self.rx_packet_counter.checked_sub(1).ok_or_else(|| {
                "encrypted BLE retransmission precedes packet counter zero".to_owned()
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
        .map_err(|error| format!("encrypted BLE RX counter {counter}: {error}"))
    }

    pub(super) fn native_rx_packet_counter(&self, received: &[u8]) -> Result<Option<u64>, String> {
        if !self.rx_encryption_active() {
            return Ok(None);
        }
        let Some(header) = received.first().copied() else {
            return Err("encrypted BLE connection PDU has no header".to_owned());
        };
        let received_new = (header & (1 << 3) != 0) == self.expected_rx_sn;
        if received_new {
            Ok(Some(self.rx_packet_counter))
        } else {
            self.rx_packet_counter
                .checked_sub(1)
                .map(Some)
                .ok_or_else(|| {
                    "encrypted BLE retransmission precedes packet counter zero".to_owned()
                })
        }
    }

    pub(super) fn observe_security_ecb(
        &mut self,
        input: [u8; 16],
        output: [u8; 16],
    ) -> Result<bool, String> {
        let Some(skd) = self.encryption_skd else {
            return Ok(false);
        };
        if self.encryption_phase == C6BleEncryptionPhase::EncReqReceived {
            let mut reversed_skdm = skd[..8].to_vec();
            reversed_skdm.reverse();
            if input[8..] != reversed_skdm {
                return Ok(false);
            }
            if self.session_key.is_some() {
                return Err("duplicate BLE session-key ECB before LL_ENC_RSP".to_owned());
            }
            let mut firmware_skd = input;
            firmware_skd.reverse();
            self.encryption_skd = Some(firmware_skd);
            self.session_key = Some(output);
            return Ok(true);
        }
        let mut expected_input = skd;
        expected_input.reverse();
        if input != expected_input {
            return Ok(false);
        }
        if self.encryption_phase != C6BleEncryptionPhase::EncRspSent || self.session_key.is_some() {
            return Err(format!(
                "BLE session-key ECB completed during {:?}",
                self.encryption_phase
            ));
        }
        self.session_key = Some(output);
        self.encryption_phase = C6BleEncryptionPhase::SessionKeyReady;
        Ok(true)
    }

    pub(super) fn observe_native_ccm(
        &mut self,
        decrypt: bool,
        peripheral_to_central: bool,
        key: &[u8; 16],
        iv: &[u8; 8],
        packet_counter: u64,
    ) -> Result<bool, String> {
        if self.session_key.as_ref() != Some(key) || self.encryption_iv.as_ref() != Some(iv) {
            return Ok(false);
        }
        if decrypt {
            if peripheral_to_central {
                return Err("native BLE RX CCM used the peripheral-to-central direction".to_owned());
            }
            let expected_counter = self.pending_native_rx_counter.ok_or_else(|| {
                "native BLE RX CCM ran without a pending encrypted reception".to_owned()
            })?;
            if packet_counter != expected_counter {
                return Err(format!(
                    "native BLE RX CCM counter {packet_counter} differs from pending counter {expected_counter}"
                ));
            }
            self.pending_native_rx_counter = None;
        } else {
            if !peripheral_to_central {
                return Err("native BLE TX CCM used the central-to-peripheral direction".to_owned());
            }
            if packet_counter != self.tx_packet_counter {
                return Err(format!(
                    "native BLE TX CCM counter {packet_counter} differs from link counter {}",
                    self.tx_packet_counter
                ));
            }
        }
        Ok(true)
    }

    fn observe_received_control(&mut self, received: &[u8]) -> Result<(), String> {
        if received.first().is_none_or(|header| header & 3 != 3) || received.len() < 3 {
            return Ok(());
        }
        match received[2] {
            0x03 => {
                if received.len() != 25
                    || self.encryption_phase != C6BleEncryptionPhase::Unencrypted
                {
                    return Err(format!(
                        "LL_ENC_REQ length {} received during {:?}",
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
                self.encryption_phase = C6BleEncryptionPhase::EncReqReceived;
            }
            0x06 => {
                if received.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::StartReqSent
                {
                    return Err(format!(
                        "LL_START_ENC_RSP received during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::StartRspReceived;
            }
            0x04 | 0x05 => {
                return Err(format!(
                    "central sent peripheral-role encryption opcode {:#04x}",
                    received[2]
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn observe_firmware_control(&mut self, response: &[u8]) -> Result<(), String> {
        if response.first().is_none_or(|header| header & 3 != 3) || response.len() < 3 {
            return Ok(());
        }
        match response[2] {
            0x04 => {
                if response.len() != 15
                    || self.encryption_phase != C6BleEncryptionPhase::EncReqReceived
                {
                    return Err(format!(
                        "LL_ENC_RSP length {} emitted during {:?}",
                        response.len(),
                        self.encryption_phase
                    ));
                }
                let skd = self
                    .encryption_skd
                    .as_mut()
                    .expect("LL_ENC_REQ established SKD storage");
                if self.session_key.is_some() && skd[8..] != response[3..11] {
                    return Err(
                        "LL_ENC_RSP SKDs differs from firmware-programmed ECB input".to_owned()
                    );
                }
                skd[8..].copy_from_slice(&response[3..11]);
                let iv = self
                    .encryption_iv
                    .as_mut()
                    .expect("LL_ENC_REQ established IV storage");
                iv[4..].copy_from_slice(&response[11..15]);
                self.encryption_phase = if self.session_key.is_some() {
                    C6BleEncryptionPhase::SessionKeyReady
                } else {
                    C6BleEncryptionPhase::EncRspSent
                };
            }
            0x05 => {
                if response.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::SessionKeyReady
                {
                    return Err(format!(
                        "LL_START_ENC_REQ emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::StartReqSent;
            }
            0x06 => {
                if response.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::StartRspReceived
                {
                    return Err(format!(
                        "LL_START_ENC_RSP emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::Encrypted;
            }
            0x03 => {
                return Err("peripheral firmware emitted central-role LL_ENC_REQ".to_owned());
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn peripheral_response(
        &mut self,
        received: &[u8],
        firmware_response: Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>, String> {
        // Validation decrypts a private copy. The native RX DMA retains the
        // over-air ciphertext so the genuine controller executes its own CCM
        // path through the modeled modem-security ECB peripheral.
        let native_rx_counter = self.native_rx_packet_counter(received)?;
        let decoded_received = self.decode_received(received)?;
        self.pending_native_rx_counter = native_rx_counter;
        let received = decoded_received.as_slice();
        let received_header = received.first().copied().unwrap_or_default();
        let received_sn = received_header & (1 << 3) != 0;
        let received_nesn = received_header & (1 << 2) != 0;

        let mut acknowledged_tx = false;
        if self.awaiting_tx_ack && received_nesn != self.tx_sn {
            if self.last_tx_encrypted {
                self.tx_packet_counter = self
                    .tx_packet_counter
                    .checked_add(1)
                    .ok_or_else(|| "BLE TX packet counter overflow".to_owned())?;
            }
            self.awaiting_tx_ack = false;
            self.tx_sn = !self.tx_sn;
            self.last_tx = None;
            self.last_tx_encrypted = false;
            acknowledged_tx = true;
        }
        let received_new = received_sn == self.expected_rx_sn;
        if received_new {
            let received_encrypted = self.rx_encryption_active();
            self.expected_rx_sn = !self.expected_rx_sn;
            self.observe_received_control(received)?;
            if received_header & 3 == 3
                && received.len() >= 7
                && received.get(1).copied() == Some(5)
                && received.get(2).copied() == Some(0x18)
            {
                let central_tx = received[3];
                let central_rx = received[4];
                let instant = u16::from_le_bytes([received[5], received[6]]);
                let select_phy = |requested, current| match requested {
                    0 => Some(current),
                    1 => Some("ble-1m"),
                    2 => Some("ble-2m"),
                    3 => Some("ble-coded"),
                    _ => None,
                };
                let Some(rx_phy) = select_phy(central_tx, self.rx_phy) else {
                    return Err(format!("invalid central TX PHY value {central_tx}"));
                };
                let Some(tx_phy) = select_phy(central_rx, self.tx_phy) else {
                    return Err(format!("invalid central RX PHY value {central_rx}"));
                };
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
                self.pending_phy_update = Some(C6BlePendingPhyUpdate {
                    instant,
                    tx_phy,
                    rx_phy,
                });
            }
            if received_encrypted {
                self.rx_packet_counter = self
                    .rx_packet_counter
                    .checked_add(1)
                    .ok_or_else(|| "BLE RX packet counter overflow".to_owned())?;
            }
        }

        let hardware_start_response =
            acknowledged_tx && self.encryption_phase == C6BleEncryptionPhase::StartRspReceived;
        let stale_firmware_start_response = acknowledged_tx
            && self.encryption_phase == C6BleEncryptionPhase::Encrypted
            && firmware_response
                .as_ref()
                .is_some_and(|pdu| pdu.get(2) == Some(&0x06));
        let mut response = if self.awaiting_tx_ack {
            self.last_tx.clone()
        } else if hardware_start_response {
            // C6 baseband completes the encryption-start exchange within the
            // continued event, before task-context firmware can advance its
            // TX list from LL_START_ENC_REQ.
            Some(vec![3, 1, 0x06])
        } else if stale_firmware_start_response {
            // The next event sees the just-acknowledged control allocation
            // until recycle advances it. Hardware emits the required empty
            // encrypted acknowledgement instead of retransmitting it.
            Some(vec![1, 0])
        } else {
            firmware_response.or_else(|| Some(vec![1, 0]))
        };
        let retransmission = self.awaiting_tx_ack;
        if let Some(pdu) = response.as_mut()
            && pdu.len() >= 2
        {
            let response_was_encrypted = !retransmission && self.tx_encryption_active();
            if !retransmission {
                // Native TX DMA is plaintext. The baseband applies CCM only
                // after firmware control processing and header sequencing.
                self.observe_firmware_control(pdu)?;
            }
            // LLID and MD come from the firmware buffer (LLID=1 for the
            // hardware-synthesized empty PDU). NESN acknowledges the next
            // expected central SN, while SN remains stable until the central
            // acknowledges this peripheral PDU.
            pdu[0] = (pdu[0] & !0x0c)
                | (u8::from(self.expected_rx_sn) << 2)
                | (u8::from(self.tx_sn) << 3);
            if response_was_encrypted {
                let (key, iv) = self.encryption_material()?;
                *pdu = ble_link_encrypt_pdu(
                    &key,
                    &iv,
                    self.tx_packet_counter,
                    BleLinkDirection::PeripheralToCentral,
                    pdu,
                )
                .map_err(|error| {
                    format!(
                        "native BLE TX CCM counter {} failed: {error}",
                        self.tx_packet_counter
                    )
                })?;
                self.last_tx_encrypted = true;
            }
            self.awaiting_tx_ack = true;
            self.last_tx = Some(pdu.clone());
        }
        Ok(response)
    }
}

impl RiscVMachine {
    fn reset_coexistence(&mut self) -> Result<(), MachineError> {
        let active_airtime = self
            .radio_coexistence
            .as_ref()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .owner()
            .is_some_and(|(_, end)| end > self.now);
        if active_airtime {
            if let Some((_, transmission)) = self.radio_coexistence_transmission.take() {
                self.radio_medium
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio medium")
                    .truncate(transmission, self.now)?;
            }
        } else {
            self.radio_coexistence_transmission = None;
        }
        self.radio_coexistence
            .as_mut()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .reset(self.now)?;
        Ok(())
    }

    fn power_down_coexistence(&mut self) -> Result<(), MachineError> {
        let Some((grant, _, end)) = self
            .radio_coexistence
            .as_ref()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .active_grant()
        else {
            return Ok(());
        };
        if end <= self.now {
            return Ok(());
        }
        let prior = self.radio_coexistence_transmission.take();
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Coexistence,
                RadioLegalityRule::CoexistenceOwnership,
                prior.is_some_and(|(mapped, _)| mapped == grant),
                self.now,
                format!("power-gated grant {grant:?} has no matching RF transmission"),
            )?;
        let (_, transmission) = prior.expect("validated coexistence transmission exists");
        self.radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium")
            .truncate(transmission, self.now)?;
        self.radio_coexistence
            .as_mut()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .power_down(self.now)?;
        Ok(())
    }

    fn apply_coexistence_preemption(
        &mut self,
        preempted: Option<CoexistenceGrantId>,
    ) -> Result<(), MachineError> {
        let Some(preempted) = preempted else {
            return Ok(());
        };
        let prior = self.radio_coexistence_transmission.take();
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Coexistence,
                RadioLegalityRule::CoexistenceOwnership,
                prior.is_some_and(|(grant, _)| grant == preempted),
                self.now,
                format!("preempted grant {preempted:?} has no matching RF transmission"),
            )?;
        let (_, transmission) = prior.expect("validated coexistence transmission exists");
        self.radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium")
            .truncate(transmission, self.now)?;
        Ok(())
    }

    fn record_coexistence_transmission(
        &mut self,
        grant: CoexistenceGrantId,
        transmission: TransmissionId,
    ) {
        self.radio_coexistence_transmission = Some((grant, transmission));
    }

    /// Returns the C6 functional Wi-Fi engine when its clock/reset domain is ready.
    pub fn wifi_engine(&mut self) -> Result<&mut WifiEngine, MachineError> {
        let ready = self
            .esp32c6_peripherals
            .as_ref()
            .is_some_and(|handles| handles.modem.wifi_ready());
        if !ready {
            return Err(MachineError::RadioNotReady("Wi-Fi"));
        }
        self.radio_wifi
            .as_mut()
            .ok_or(MachineError::UnsupportedTarget(self.target))
    }

    /// Returns the C6 functional BLE HCI controller when its domain is ready.
    pub fn ble_controller(&mut self) -> Result<&mut BleController, MachineError> {
        let ready = self
            .esp32c6_peripherals
            .as_ref()
            .is_some_and(|handles| handles.modem.ble_ready());
        if !ready {
            return Err(MachineError::RadioNotReady("Bluetooth LE"));
        }
        self.radio_ble
            .as_mut()
            .ok_or(MachineError::UnsupportedTarget(self.target))
    }

    /// Injects one explicit packet into the host-isolated deterministic medium.
    ///
    /// The packet can only reach an armed emulated receiver. This method never
    /// opens a socket or forwards traffic to a host network interface.
    pub fn inject_radio_frame(
        &mut self,
        protocol: RadioProtocol,
        spectrum: Spectrum,
        phy: impl Into<String>,
        bytes: Vec<u8>,
        power_dbm: i16,
    ) -> Result<(), MachineError> {
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
    ) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let medium = self
            .radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium");
        medium.tune_receiver(Receiver {
            node: EMULATED_NODE,
            protocol,
            spectrum,
            sensitivity_dbm: -100,
        })?;
        let duration = frame_duration(bytes.len());
        medium.transmit(TxRequest {
            source: HOST_NODE,
            start: at,
            end: at
                .checked_add(duration)
                .map_err(|_| MachineError::TimeOverflow)?,
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

    /// Returns a versioned snapshot of all RF submissions and outcomes.
    pub fn radio_replay_artifact(&self) -> Option<ReplayArtifact> {
        self.radio_medium.as_ref().map(|medium| {
            let mut artifact =
                ReplayArtifact::new(medium.profile().clone(), medium.events().to_vec());
            if let Some(arbiter) = &self.radio_coexistence {
                artifact = artifact.with_coexistence_events(arbiter.events().to_vec());
            }
            artifact
        })
    }

    pub(super) fn service_radio(&mut self) -> Result<u64, MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Ok(0);
        }
        let (modem, ble_modem, ble_baseband, ble_control, ieee802154, interrupt_matrix, wifi_mac) = {
            let Some(handles) = self.esp32c6_peripherals.as_ref() else {
                return Ok(0);
            };
            (
                handles.modem.clone(),
                handles.ble_modem.clone(),
                handles.ble_baseband.clone(),
                handles.ble_control.clone(),
                handles.ieee802154.clone(),
                handles.interrupt_matrix.clone(),
                handles.wifi_mac.clone(),
            )
        };
        let reset_generations = modem.reset_generations();
        let reset_changed = std::array::from_fn::<_, 4, _>(|index| {
            reset_generations[index] != self.radio_c6_reset_generations[index]
        });
        if reset_changed.iter().any(|changed| *changed) {
            self.reset_coexistence()?;
        }
        if self
            .radio_coexistence
            .as_ref()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .active_grant()
            .is_some_and(|(_, protocol, _)| match protocol {
                RadioProtocol::Wifi => !modem.wifi_ready(),
                RadioProtocol::BluetoothLe => !modem.ble_ready(),
                RadioProtocol::Ieee802154 => !modem.ieee802154_ready(),
            })
        {
            self.power_down_coexistence()?;
        }
        if reset_changed[1] {
            self.radio_c6_ble_receptions.clear();
            self.radio_c6_ble_completion_anchors.clear();
            self.radio_c6_ble_schedule_records.clear();
            self.radio_c6_pending_ble_transmissions.clear();
            self.radio_c6_ble_link_sequences.clear();
        }
        if reset_changed[2] {
            self.radio_pending_ieee802154_tx.clear();
            self.radio_pending_ieee802154_ack.clear();
            self.radio_pending_ieee802154_cca = None;
        }
        self.radio_c6_reset_generations = reset_generations;
        let legality = self
            .radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator");
        legality.observe_domain(
            RadioSubsystem::Wifi,
            modem.wifi_ready(),
            Some(reset_generations[0]),
            self.now,
        )?;
        legality.observe_domain(
            RadioSubsystem::BluetoothLe,
            modem.ble_ready(),
            Some(reset_generations[1]),
            self.now,
        )?;
        legality.observe_domain(
            RadioSubsystem::Ieee802154,
            modem.ieee802154_ready(),
            Some(reset_generations[2]),
            self.now,
        )?;
        legality.observe_domain(
            RadioSubsystem::Coexistence,
            modem.coexistence_ready(),
            Some(reset_generations[3]),
            self.now,
        )?;
        let awaiting_ack_before_poll = ieee802154.awaiting_ack_sequence().is_some();
        ieee802154.poll(self.now);
        if awaiting_ack_before_poll && ieee802154.awaiting_ack_sequence().is_none() {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .transition_activity(
                    RadioSubsystem::Ieee802154,
                    RadioActivity::AwaitingAck,
                    RadioActivity::Idle,
                    self.now,
                )?;
        }
        ble_baseband.advance_to(self.now);
        let mut events = self.service_native_ble_completions(&ble_baseband)?;
        if ble_baseband.take_stop_request() {
            self.radio_c6_ble_receptions.clear();
            self.radio_c6_ble_completion_anchors.clear();
            self.radio_c6_ble_schedule_records.clear();
            self.radio_c6_pending_ble_transmissions.clear();
            self.radio_c6_ble_link_sequences.clear();
        }
        events = events.saturating_add(self.service_ble_security_dma(&ble_control)?);
        events = events.saturating_add(self.service_native_ble_schedules(&ble_baseband)?);
        events = events.saturating_add(self.service_native_ble_conflicts(&ble_baseband)?);
        self.sync_ieee802154_configuration(&ieee802154)?;

        self.radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium")
            .advance_to(self.now)?;
        self.radio_coexistence
            .as_mut()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .advance_to(self.now)?;
        events = events.saturating_add(self.submit_pending_native_ble_frames()?);
        if modem.ble_ready() {
            self.radio_ble
                .as_mut()
                .expect("ESP32-C6 machine has a BLE controller")
                .advance_to(self.now);
        }
        if modem.wifi_ready() {
            self.radio_wifi
                .as_mut()
                .expect("ESP32-C6 machine has a Wi-Fi engine")
                .advance_to(self.now);
        }
        events = events.saturating_add(self.complete_medium_receptions(
            &ieee802154,
            &wifi_mac,
            &ble_baseband,
        )?);
        if self
            .radio_pending_ieee802154_cca
            .is_some_and(|at| at <= self.now)
        {
            self.radio_pending_ieee802154_cca = None;
            self.complete_ieee802154_cca_tx(&modem, &ieee802154)?;
            events = events.saturating_add(1);
        }
        let completed_acks = self
            .radio_pending_ieee802154_ack
            .iter()
            .filter(|end| **end <= self.now)
            .count();
        if completed_acks != 0 {
            ieee802154.complete_ack_tx();
            self.radio_pending_ieee802154_ack
                .retain(|end| *end > self.now);
            events = events.saturating_add(completed_acks as u64);
        }
        let completed = self
            .radio_pending_ieee802154_tx
            .iter()
            .filter(|(_, _, end, _)| *end <= self.now)
            .count();
        if completed != 0 {
            let completed_ack_sequences = self
                .radio_pending_ieee802154_tx
                .iter()
                .filter(|(_, _, end, _)| *end <= self.now)
                .map(|(_, _, _, sequence)| *sequence)
                .collect::<Vec<_>>();
            for sequence in completed_ack_sequences {
                if let Some(sequence) = sequence {
                    ieee802154.complete_tx_expect_ack(sequence);
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .transition_activity(
                            RadioSubsystem::Ieee802154,
                            RadioActivity::Transmit,
                            RadioActivity::AwaitingAck,
                            self.now,
                        )?;
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .tune_receiver(Receiver {
                            node: EMULATED_NODE,
                            protocol: RadioProtocol::Ieee802154,
                            spectrum: ieee802154_spectrum(ieee802154.channel()),
                            sensitivity_dbm: -100,
                        })?;
                } else {
                    ieee802154.complete_tx();
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .transition_activity(
                            RadioSubsystem::Ieee802154,
                            RadioActivity::Transmit,
                            RadioActivity::Idle,
                            self.now,
                        )?;
                }
            }
            events = events.saturating_add(completed as u64);
            self.radio_pending_ieee802154_tx
                .retain(|(_, _, end, _)| *end > self.now);
        }

        while let Some(command) = ieee802154.take_command() {
            events = events.saturating_add(1);
            match command {
                EspIeee802154Command::TxStart => {
                    if !modem.ieee802154_ready() {
                        ieee802154.abort(true, 17);
                        continue;
                    }
                    self.submit_ieee802154_tx(&ieee802154)?;
                }
                EspIeee802154Command::CcaTxStart => {
                    if !modem.ieee802154_ready() {
                        ieee802154.abort(true, 17);
                        continue;
                    }
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .begin_activity(
                            RadioSubsystem::Ieee802154,
                            RadioActivity::ClearChannelAssessment,
                            self.now,
                        )?;
                    let duration = u64::from(ieee802154.configuration().ed_duration_symbols.max(1))
                        .saturating_mul(16);
                    self.radio_pending_ieee802154_cca = Some(
                        self.now
                            .checked_add(SimDuration::from_ticks(duration))
                            .map_err(|_| MachineError::TimeOverflow)?,
                    );
                }
                EspIeee802154Command::EnergyDetectStart => {
                    if modem.ieee802154_ready() {
                        self.radio_legality
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio legality validator")
                            .begin_activity(
                                RadioSubsystem::Ieee802154,
                                RadioActivity::EnergyDetection,
                                self.now,
                            )?;
                        let spectrum = ieee802154_spectrum(ieee802154.channel());
                        let energy = self
                            .radio_medium
                            .as_ref()
                            .expect("ESP32-C6 machine has a radio medium")
                            .energy_dbm_at(EMULATED_NODE, spectrum);
                        let carrier = self
                            .radio_medium
                            .as_ref()
                            .expect("ESP32-C6 machine has a radio medium")
                            .carrier_present_at(
                                EMULATED_NODE,
                                RadioProtocol::Ieee802154,
                                spectrum,
                                -100,
                            );
                        let configuration = ieee802154.configuration();
                        let busy = self
                            .radio_ieee802154_mac
                            .as_mut()
                            .expect("ESP32-C6 machine has an IEEE 802.15.4 MAC")
                            .clear_channel_assessment_with_mode(
                                energy,
                                carrier,
                                decode_cca_mode(configuration.cca_mode),
                            );
                        ieee802154.complete_energy_detect(energy.clamp(-128, 127) as i8, busy);
                        self.radio_legality
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio legality validator")
                            .transition_activity(
                                RadioSubsystem::Ieee802154,
                                RadioActivity::EnergyDetection,
                                RadioActivity::Idle,
                                self.now,
                            )?;
                    } else {
                        ieee802154.abort(false, 24);
                    }
                }
                EspIeee802154Command::Stop | EspIeee802154Command::TestStop => {
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .force_idle(RadioSubsystem::Ieee802154, self.now)?;
                    self.radio_pending_ieee802154_cca = None;
                    if !self.radio_pending_ieee802154_tx.is_empty() {
                        ieee802154.abort(true, 17);
                        self.radio_pending_ieee802154_tx.clear();
                    }
                    self.radio_pending_ieee802154_ack.clear();
                }
                EspIeee802154Command::RxStart
                | EspIeee802154Command::TestTxStart
                | EspIeee802154Command::TestRxStart
                | EspIeee802154Command::Timer0Start
                | EspIeee802154Command::Timer0Stop
                | EspIeee802154Command::Timer1Start
                | EspIeee802154Command::Timer1Stop => {}
            }
        }
        events = events.saturating_add(self.submit_native_wifi_frames(&wifi_mac)?);
        events = events.saturating_add(self.submit_protocol_engine_frames()?);
        let wifi_pending = wifi_mac.interrupt_pending()
            || self.radio_wifi.as_ref().is_some_and(WifiEngine::has_rx);
        let ble_pending = ble_baseband.interrupt_pending()
            || self
                .radio_ble
                .as_ref()
                .is_some_and(BleController::has_h4_output);
        let ieee802154_pending = ieee802154.interrupt_pending();
        let legality = self
            .radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator");
        legality.observe_interrupt(RadioSubsystem::Wifi, wifi_pending, self.now)?;
        legality.observe_interrupt(RadioSubsystem::BluetoothLe, ble_pending, self.now)?;
        legality.observe_interrupt(RadioSubsystem::Ieee802154, ieee802154_pending, self.now)?;
        interrupt_matrix.set_source(0, wifi_pending);
        interrupt_matrix.set_source(4, ble_pending);
        // Source 5 is the separately exposed BT_BB line. Current C6 controller
        // firmware installs its combined native PHY ISR on BT_MAC source 4,
        // while freestanding stacks may route the baseband source directly.
        interrupt_matrix.set_source(5, ble_baseband.interrupt_pending());
        // Genuine C6 BLE sleep firmware routes the modem wake compare through
        // LP_TIMER source 7 before enabling the controller's BT_MAC line.
        interrupt_matrix.set_source(7, ble_modem.interrupt_pending(self.now));
        interrupt_matrix.set_source(12, ieee802154_pending);
        Ok(events)
    }

    fn submit_native_wifi_frames(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
    ) -> Result<u64, MachineError> {
        let mut submitted = 0_u64;
        while let Some(descriptor) = wifi_mac.take_tx_descriptor() {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .validate_dma(
                    RadioSubsystem::Wifi,
                    RadioDmaDirection::Transmit,
                    descriptor.address,
                    4,
                    12,
                    12,
                    self.now,
                )?;
            let buffer = self.bus.read(
                u64::from(descriptor.address.wrapping_add(4)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let buffer = buffer as u32;
            let wire_length = self.bus.read(
                u64::from(buffer),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let wire_length = wire_length as usize;
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    remu_radio::RadioLegalityRule::DmaLength,
                    wire_length > 4,
                    self.now,
                    format!(
                        "TX DMA wire length {wire_length} does not contain a MAC frame and 4-byte FCS"
                    ),
                )?;
            // Genuine net80211 descriptors include the hardware-generated FCS
            // in their wire length. Guest memory and the shared RF medium carry
            // only the MAC frame; receive DMA provides its own four-byte area.
            let length = wire_length - 4;
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .validate_dma(
                    RadioSubsystem::Wifi,
                    RadioDmaDirection::Transmit,
                    buffer.wrapping_add(8),
                    4,
                    length,
                    4095,
                    self.now,
                )?;
            let bytes = self.radio_read_guest_bytes(buffer.wrapping_add(8), length)?;
            let duration = frame_duration(bytes.len());
            let decision = self
                .radio_coexistence
                .as_mut()
                .expect("ESP32-C6 machine has a coexistence arbiter")
                .request(CoexistenceRequest {
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
                continue;
            };
            self.apply_coexistence_preemption(preempted)?;
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .validate_coexistence_ownership(
                    RadioSubsystem::Wifi,
                    RadioProtocol::Wifi,
                    granted_protocol,
                    self.now,
                )?;
            let transmission = self
                .radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .transmit(TxRequest {
                    source: EMULATED_NODE,
                    start: self.now,
                    end: self
                        .now
                        .checked_add(duration)
                        .map_err(|_| MachineError::TimeOverflow)?,
                    power_dbm: 0,
                    frame: RadioFrame {
                        protocol: RadioProtocol::Wifi,
                        spectrum: Spectrum::new(2_412_000, 20_000),
                        phy: "wifi-ht20".to_owned(),
                        bytes,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    fn complete_ieee802154_cca_tx(
        &mut self,
        modem: &remu_devices::EspC6ModemHandle,
        handle: &EspIeee802154Handle,
    ) -> Result<(), MachineError> {
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .transition_activity(
                RadioSubsystem::Ieee802154,
                RadioActivity::ClearChannelAssessment,
                RadioActivity::Idle,
                self.now,
            )?;
        if !modem.ieee802154_ready() {
            handle.abort(true, 17);
            return Ok(());
        }
        let channel = handle.channel();
        if !(11..=26).contains(&channel) {
            handle.abort(true, 17);
            return Ok(());
        }
        let spectrum = ieee802154_spectrum(channel);
        let energy = self
            .radio_medium
            .as_ref()
            .expect("ESP32-C6 machine has a radio medium")
            .energy_dbm_at(EMULATED_NODE, spectrum);
        let carrier = self
            .radio_medium
            .as_ref()
            .expect("ESP32-C6 machine has a radio medium")
            .carrier_present_at(EMULATED_NODE, RadioProtocol::Ieee802154, spectrum, -100);
        let configuration = handle.configuration();
        let busy = self
            .radio_ieee802154_mac
            .as_mut()
            .expect("ESP32-C6 machine has an IEEE 802.15.4 MAC")
            .clear_channel_assessment_with_mode(
                energy,
                carrier,
                decode_cca_mode(configuration.cca_mode),
            );
        if !busy {
            return self.submit_ieee802154_tx(handle);
        }
        // CCA_TX_START is one peripheral operation. Guest firmware owns any
        // deterministic CSMA-CA retry and backoff policy after a busy result.
        handle.record_cca_busy();
        Ok(())
    }

    fn submit_ieee802154_tx(&mut self, handle: &EspIeee802154Handle) -> Result<(), MachineError> {
        let (tx_address, _) = handle.dma_addresses();
        let length = self.bus.read(
            u64::from(tx_address),
            AccessWidth::Byte,
            AccessKind::Read,
            self.now,
        )? as usize;
        // The vendor DMA length is the over-the-air PSDU length, including
        // the two-byte hardware-generated FCS. Firmware reserves those two
        // bytes but does not initialize them.
        if !(3..=127).contains(&length) {
            handle.abort(true, 17);
            return Ok(());
        }
        let mut bytes = self.radio_read_guest_bytes(tx_address.wrapping_add(1), length - 2)?;
        let configuration = handle.configuration();
        if configuration.transmit_security {
            let payload_offset = usize::from(configuration.security_offset);
            bytes = match self
                .radio_ieee802154_mac
                .as_mut()
                .expect("ESP32-C6 machine has an IEEE 802.15.4 MAC")
                .protect_transmit_frame(
                    &bytes,
                    payload_offset,
                    configuration.security_key,
                    ExtendedAddress(configuration.security_address),
                ) {
                Ok(bytes) => bytes,
                Err(Ieee802154Error::InvalidLength(_)) => {
                    handle.record_security_failure(4);
                    return Ok(());
                }
                Err(Ieee802154Error::SecurityNotEnabled) => {
                    handle.record_security_failure(1);
                    return Ok(());
                }
                Err(Ieee802154Error::InvalidSecurityLevel(_)) => {
                    handle.record_security_failure(2);
                    return Ok(());
                }
                Err(Ieee802154Error::InvalidSecurityOffset(_)) => {
                    handle.record_security_failure(4);
                    return Ok(());
                }
                Err(Ieee802154Error::SecurityCounterSuppressed) => {
                    handle.record_security_failure(5);
                    return Ok(());
                }
                Err(Ieee802154Error::AuthenticationFailed) => {
                    handle.record_security_failure(4);
                    return Ok(());
                }
                Err(Ieee802154Error::MalformedHeader | Ieee802154Error::MalformedSecurity) => {
                    handle.record_security_failure(3);
                    return Ok(());
                }
                Err(_) => {
                    handle.record_security_failure(1);
                    return Ok(());
                }
            };
        }
        let bytes = Ieee802154Mac::with_fcs(bytes);
        let channel = handle.channel();
        if !(11..=26).contains(&channel) {
            handle.abort(true, 17);
            return Ok(());
        }
        let spectrum = ieee802154_spectrum(channel);
        let duration = frame_duration(bytes.len());
        let end = self
            .now
            .checked_add(duration)
            .map_err(|_| MachineError::TimeOverflow)?;
        let decision = self
            .radio_coexistence
            .as_mut()
            .expect("ESP32-C6 machine has a coexistence arbiter")
            .request(CoexistenceRequest {
                protocol: RadioProtocol::Ieee802154,
                start: self.now,
                duration,
                priority: handle.coexistence_priority(),
                preemptible: true,
            })?;
        let CoexistenceDecision::Granted {
            id: grant,
            protocol: granted_protocol,
            preempted,
            ..
        } = decision
        else {
            handle.abort(true, 18);
            return Ok(());
        };
        self.apply_coexistence_preemption(preempted)?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .validate_coexistence_ownership(
                RadioSubsystem::Ieee802154,
                RadioProtocol::Ieee802154,
                granted_protocol,
                self.now,
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .begin_activity(
                RadioSubsystem::Ieee802154,
                RadioActivity::Transmit,
                self.now,
            )?;
        let ack_sequence = (configuration.automatic_ack_receive
            && bytes.len() >= 3
            && u16::from_le_bytes([bytes[0], bytes[1]]) & (1 << 5) != 0)
            .then_some(bytes[2]);
        let id = self
            .radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium")
            .transmit(TxRequest {
                source: EMULATED_NODE,
                start: self.now,
                end,
                power_dbm: decode_tx_power(handle.tx_power()),
                frame: RadioFrame {
                    protocol: RadioProtocol::Ieee802154,
                    spectrum,
                    phy: "ieee802154-oqpsk-250k".to_owned(),
                    bytes,
                    origin: FrameOrigin::Emulated,
                },
            })?;
        self.record_coexistence_transmission(grant, id);
        self.radio_pending_ieee802154_tx
            .push((id, grant, end, ack_sequence));
        Ok(())
    }
}
include!("radio_receive.rs");
include!("radio_ble.rs");

fn decode_cca_mode(mode: u8) -> Ieee802154CcaMode {
    match mode & 3 {
        0 => Ieee802154CcaMode::Carrier,
        1 => Ieee802154CcaMode::Energy,
        2 => Ieee802154CcaMode::CarrierOrEnergy,
        _ => Ieee802154CcaMode::CarrierAndEnergy,
    }
}

fn decode_tx_power(encoded: u8) -> i16 {
    let encoded = encoded & 0x1f;
    if encoded & 0x10 != 0 {
        i16::from(encoded) - 32
    } else {
        i16::from(encoded)
    }
}
