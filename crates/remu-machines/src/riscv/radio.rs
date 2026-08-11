use super::{MachineError, RiscVMachine};
use crate::TargetId;
use remu_core::{AccessKind, AccessWidth, Bus, SimDuration};
use remu_devices::{
    EspC6BleBasebandHandle, EspC6BleControlHandle, EspIeee802154Command, EspIeee802154Handle,
};
use remu_radio::{
    BleController, CoexistenceDecision, CoexistenceGrantId, CoexistenceRequest, DeliveryOutcome,
    ExtendedAddress, FrameOrigin, Ieee802154CcaMode, Ieee802154Error, Ieee802154Mac,
    Ieee802154RxOutcome, MediumEvent, NodeId, PanInterface, RadioActivity, RadioDmaDirection,
    RadioFrame, RadioLegalityRule, RadioProtocol, RadioSubsystem, Receiver, ReplayArtifact,
    ShortAddress, Spectrum, TransmissionId, TxRequest, WifiEngine,
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
    rx_buffer_identifier: u16,
}

pub(super) struct PendingNativeBleTransmission {
    start: remu_core::SimTime,
    spectrum: Spectrum,
    phy: &'static str,
    bytes: Vec<u8>,
    response: Option<PendingNativeBleReception>,
}

#[derive(Default)]
pub(super) struct C6BleLinkSequence {
    pub(super) expected_rx_sn: bool,
    pub(super) tx_sn: bool,
    pub(super) awaiting_tx_ack: bool,
    pub(super) last_tx: Option<Vec<u8>>,
}

impl C6BleLinkSequence {
    pub(super) fn peripheral_response(
        &mut self,
        received_header: u8,
        firmware_response: Option<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let received_sn = received_header & (1 << 3) != 0;
        let received_nesn = received_header & (1 << 2) != 0;

        if self.awaiting_tx_ack && received_nesn != self.tx_sn {
            self.awaiting_tx_ack = false;
            self.tx_sn = !self.tx_sn;
            self.last_tx = None;
        }
        if received_sn == self.expected_rx_sn {
            self.expected_rx_sn = !self.expected_rx_sn;
        }

        let mut response = if self.awaiting_tx_ack {
            self.last_tx.clone()
        } else {
            firmware_response.or_else(|| Some(vec![1, 0]))
        };
        if let Some(pdu) = response.as_mut()
            && pdu.len() >= 2
        {
            // LLID and MD come from the firmware buffer (LLID=1 for the
            // hardware-synthesized empty PDU). NESN acknowledges the next
            // expected central SN, while SN remains stable until the central
            // acknowledges this peripheral PDU.
            pdu[0] = (pdu[0] & !0x0c)
                | (u8::from(self.expected_rx_sn) << 2)
                | (u8::from(self.tx_sn) << 3);
            self.awaiting_tx_ack = true;
            self.last_tx = Some(pdu.clone());
        }
        response
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
        let (modem, ble_baseband, ble_control, ieee802154, interrupt_matrix, wifi_mac) = {
            let Some(handles) = self.esp32c6_peripherals.as_ref() else {
                return Ok(0);
            };
            (
                handles.modem.clone(),
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
        if !(1..=127).contains(&length) {
            handle.abort(true, 17);
            return Ok(());
        }
        let mut bytes = self.radio_read_guest_bytes(tx_address.wrapping_add(1), length)?;
        let configuration = handle.configuration();
        if configuration.transmit_security {
            let Some(payload_offset) = usize::from(configuration.security_offset).checked_sub(1)
            else {
                handle.record_security_failure(2);
                return Ok(());
            };
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
