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
    RadioDmaDirection, RadioFrame, RadioLegalityRule, RadioPeer, RadioProtocol, RadioSubsystem,
    Receiver, ReplayArtifact, ShortAddress, Spectrum, TransmissionId, TxRequest, WifiEngine,
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

include!("radio_ble_sequence.rs");

impl RiscVMachine {
    /// Attaches a deterministic external peer to the ESP32-C6 isolated RF
    /// medium. The peer observes emitted frames only and cannot access machine
    /// state.
    pub fn set_radio_peer(&mut self, peer: Box<dyn RadioPeer>) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        self.radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium")
            .set_peer(peer);
        Ok(())
    }

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

    pub(super) fn c6_wifi_rf_airtime(&mut self) -> Result<(Spectrum, i16), MachineError> {
        let domain_ready = self
            .esp32c6_peripherals
            .as_ref()
            .is_some_and(|handles| handles.modem.wifi_ready());
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::DomainReady,
                domain_ready,
                self.now,
                "Wi-Fi RF airtime requested while its APB/MAC clock domain is disabled",
            )?;
        let snapshot = self
            .esp32c6_peripherals
            .as_ref()
            .expect("ESP32-C6 machine has peripheral handles")
            .wifi_rf
            .wifi_rf_snapshot();
        let center_khz = snapshot.center_khz();
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfPllLock,
                snapshot.pll_locked,
                self.now,
                "Wi-Fi airtime requested before an RFPLL channel strobe completed",
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfCalibration,
                snapshot.calibration_valid,
                self.now,
                format!(
                    "Wi-Fi RF calibration {:?} is absent or stale for configuration generation {} in reset generation {}",
                    snapshot.calibrated_generation,
                    snapshot.calibration_generation,
                    snapshot.generation,
                ),
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfChannel,
                center_khz.is_some(),
                self.now,
                format!("unsupported Wi-Fi RF channel {:?}", snapshot.channel),
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfPower,
                snapshot.gain_entries == 43
                    && snapshot
                        .power_qdbm
                        .is_some_and(|power| (8..=84).contains(&power) && power % 4 == 0),
                self.now,
                format!(
                    "Wi-Fi RF gain table has {} entries and power {:?} quarter-dBm",
                    snapshot.gain_entries, snapshot.power_qdbm
                ),
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfBandwidth,
                snapshot.bandwidth_khz == Some(20_000),
                self.now,
                format!(
                    "native C6 Wi-Fi RFPLL selected unsupported bandwidth {:?}",
                    snapshot.bandwidth_khz
                ),
            )?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::RfFrontend,
                snapshot.frontend_released == Some(true),
                self.now,
                format!(
                    "Wi-Fi frontend release state is {:?}",
                    snapshot.frontend_released
                ),
            )?;
        Ok((
            Spectrum::new(
                center_khz.expect("legality accepted RF channel"),
                snapshot
                    .bandwidth_khz
                    .expect("legality accepted RF bandwidth"),
            ),
            snapshot
                .power_qdbm
                .expect("legality accepted RF power profile")
                / 4,
        ))
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
        if protocol != RadioProtocol::Wifi {
            medium.tune_receiver(Receiver {
                node: EMULATED_NODE,
                protocol,
                spectrum,
                sensitivity_dbm: -100,
            })?;
        }
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
                mpdus: Vec::new(),
                origin: FrameOrigin::HostInjection,
            },
        })?;
        Ok(())
    }

    /// Schedules one explicit host Wi-Fi A-MPDU at a simulation timestamp.
    ///
    /// MPDU boundaries remain explicit so this is one native RF/coexistence
    /// operation without introducing an HLE delimiter encoding.
    pub fn inject_wifi_ampdu_at(
        &mut self,
        at: remu_core::SimTime,
        spectrum: Spectrum,
        mpdus: Vec<Vec<u8>>,
        power_dbm: i16,
    ) -> Result<(), MachineError> {
        if self.target != TargetId::Esp32c6 {
            return Err(MachineError::UnsupportedTarget(self.target));
        }
        let medium = self
            .radio_medium
            .as_mut()
            .expect("ESP32-C6 machine has a radio medium");
        let length = mpdus.iter().map(Vec::len).sum();
        let duration = frame_duration(length);
        medium.transmit(TxRequest {
            source: HOST_NODE,
            start: at,
            end: at
                .checked_add(duration)
                .map_err(|_| MachineError::TimeOverflow)?,
            power_dbm,
            frame: RadioFrame {
                protocol: RadioProtocol::Wifi,
                spectrum,
                phy: "wifi-ht20-ampdu".to_owned(),
                bytes: Vec::new(),
                mpdus,
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
        let (
            modem,
            ble_modem,
            ble_baseband,
            ble_control,
            ieee802154,
            interrupt_matrix,
            wifi_mac,
            phy,
            wifi_rf,
        ) = {
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
                handles.phy.clone(),
                handles.wifi_rf.clone(),
            )
        };
        let reset_generations = modem.reset_generations();
        let wifi_mac_reset_generation = wifi_mac.reset_generation();
        let wifi_mac_reset_changed =
            wifi_mac_reset_generation != self.radio_c6_wifi_mac_reset_generation;
        let reset_changed = std::array::from_fn::<_, 4, _>(|index| {
            reset_generations[index] != self.radio_c6_reset_generations[index]
        });
        if reset_changed.iter().any(|changed| *changed) {
            self.reset_coexistence()?;
        }
        if reset_changed[0] {
            self.radio_pending_native_wifi.clear();
            wifi_rf.invalidate_wifi_rf();
        }
        if wifi_mac_reset_changed {
            if self
                .radio_coexistence
                .as_ref()
                .expect("ESP32-C6 machine has a coexistence arbiter")
                .active_grant()
                .is_some_and(|(_, protocol, _)| protocol == RadioProtocol::Wifi)
            {
                self.reset_coexistence()?;
            }
            self.radio_pending_native_wifi.clear();
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .remove_receiver(EMULATED_NODE, RadioProtocol::Wifi);
            wifi_rf.invalidate_wifi_rf();
            self.radio_c6_wifi_mac_reset_generation = wifi_mac_reset_generation;
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
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .remove_receiver(EMULATED_NODE, RadioProtocol::Ieee802154);
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
        let crypto_state = wifi_mac.validate_crypto_key_table();
        legality.require(
            RadioSubsystem::Wifi,
            RadioLegalityRule::SchedulerState,
            crypto_state.is_ok(),
            self.now,
            crypto_state.err().unwrap_or_default(),
        )?;
        let tsf_timer_state = phy.validate_tsf_timers();
        legality.require(
            RadioSubsystem::Wifi,
            RadioLegalityRule::SchedulerState,
            tsf_timer_state.is_ok(),
            self.now,
            tsf_timer_state.err().unwrap_or_default(),
        )?;
        let tsf_timer_events = phy.advance_to(self.now);
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
        let mut events =
            tsf_timer_events.saturating_add(self.service_native_ble_completions(&ble_baseband)?);
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

        if wifi_rf.wifi_rf_snapshot().airtime_ready() {
            let spectrum = self.c6_wifi_rf_airtime()?.0;
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::Wifi,
                    spectrum,
                    sensitivity_dbm: -100,
                })?;
        } else if wifi_mac.rx_descriptor().is_some() {
            let _ = self.c6_wifi_rf_airtime()?;
            unreachable!("incomplete Wi-Fi RF state passed its legality checks");
        } else if !self
            .radio_pending_native_wifi
            .iter()
            .any(|pending| pending.expected_response.is_some())
        {
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .remove_receiver(EMULATED_NODE, RadioProtocol::Wifi);
        }

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
        events = events.saturating_add(self.complete_native_wifi_transmissions(&wifi_mac)?);
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
            .filter(|(_, _, end)| *end <= self.now)
            .count();
        if completed_acks != 0 {
            ieee802154.complete_ack_tx();
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .transition_activity(
                    RadioSubsystem::Ieee802154,
                    RadioActivity::Transmit,
                    RadioActivity::Idle,
                    self.now,
                )?;
            self.radio_pending_ieee802154_ack
                .retain(|(_, _, end)| *end > self.now);
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
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .remove_receiver(EMULATED_NODE, RadioProtocol::Ieee802154);
                    if !modem.ieee802154_ready() {
                        ieee802154.abort(true, 17);
                        continue;
                    }
                    self.submit_ieee802154_tx(&ieee802154)?;
                }
                EspIeee802154Command::CcaTxStart => {
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .remove_receiver(EMULATED_NODE, RadioProtocol::Ieee802154);
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
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .remove_receiver(EMULATED_NODE, RadioProtocol::Ieee802154);
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
                EspIeee802154Command::RxStart | EspIeee802154Command::TestRxStart => {
                    if !modem.ieee802154_ready() {
                        ieee802154.abort(false, 24);
                        continue;
                    }
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .begin_activity(
                            RadioSubsystem::Ieee802154,
                            RadioActivity::Receive,
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
                }
                EspIeee802154Command::TestTxStart
                | EspIeee802154Command::Timer0Start
                | EspIeee802154Command::Timer0Stop
                | EspIeee802154Command::Timer1Start
                | EspIeee802154Command::Timer1Stop => {}
            }
        }
        events = events.saturating_add(self.submit_native_wifi_frames(&wifi_mac)?);
        events = events.saturating_add(self.submit_protocol_engine_frames()?);
        let wifi_mac_pending = wifi_mac.interrupt_pending()
            || self.radio_wifi.as_ref().is_some_and(WifiEngine::has_rx);
        let wifi_power_pending = phy.interrupt_pending();
        let wifi_pending = wifi_mac_pending || wifi_power_pending;
        let ble_pending = ble_baseband.interrupt_pending()
            || self
                .radio_ble
                .as_ref()
                .is_some_and(BleController::has_h4_output);
        let ble_baseband_pending = ble_baseband.interrupt_pending();
        let ble_modem_pending = ble_modem.interrupt_pending(self.now);
        let ieee802154_pending = ieee802154.interrupt_pending();
        let legality = self
            .radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator");
        legality.observe_interrupt(RadioSubsystem::Wifi, wifi_pending, self.now)?;
        legality.observe_interrupt(RadioSubsystem::BluetoothLe, ble_pending, self.now)?;
        legality.observe_interrupt(RadioSubsystem::Ieee802154, ieee802154_pending, self.now)?;
        for (index, (source, line, asserted)) in [
            ("esp32c6.wifi-mac", 0_u16, wifi_mac_pending),
            ("esp32c6.wifi-power", 2, wifi_power_pending),
            ("esp32c6.bluetooth-mac", 4, ble_pending),
            ("esp32c6.bluetooth-baseband", 5, ble_baseband_pending),
            ("esp32c6.lp-timer", 7, ble_modem_pending),
            ("esp32c6.ieee802154", 12, ieee802154_pending),
        ]
        .into_iter()
        .enumerate()
        {
            if self.radio_c6_interrupt_sources[index] != asserted {
                self.bus
                    .observe_interrupt_transition(self.now, source, line, asserted);
                self.radio_c6_interrupt_sources[index] = asserted;
            }
        }
        // C6 exposes distinct native interrupt-matrix inputs for the MAC and
        // power/TSF block.  In particular, TWT compare events must reach the
        // vendor power ISR (source 2), not the packet-MAC ISR (source 0).
        interrupt_matrix.set_source(0, wifi_mac_pending);
        interrupt_matrix.set_source(2, wifi_power_pending);
        interrupt_matrix.set_source(4, ble_pending);
        // Source 5 is the separately exposed BT_BB line. Current C6 controller
        // firmware installs its combined native PHY ISR on BT_MAC source 4,
        // while freestanding stacks may route the baseband source directly.
        interrupt_matrix.set_source(5, ble_baseband_pending);
        // Genuine C6 BLE sleep firmware routes the modem wake compare through
        // LP_TIMER source 7 before enabling the controller's BT_MAC line.
        interrupt_matrix.set_source(7, ble_modem_pending);
        interrupt_matrix.set_source(12, ieee802154_pending);
        Ok(events)
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
                    mpdus: Vec::new(),
                    origin: FrameOrigin::Emulated,
                },
            })?;
        self.record_coexistence_transmission(grant, id);
        self.radio_pending_ieee802154_tx
            .push((id, grant, end, ack_sequence));
        Ok(())
    }
}
include!("radio_wifi_tx.rs");
include!("radio_wifi_completion.rs");
include!("radio_receive.rs");
include!("radio_ble.rs");

include!("radio_helpers.rs");
