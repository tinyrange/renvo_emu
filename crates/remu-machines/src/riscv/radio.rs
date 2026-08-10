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
        if reset_changed[1] {
            self.radio_c6_ble_scan = None;
            self.radio_c6_ble_completion_anchors.clear();
            self.radio_c6_ble_schedule_records.clear();
            self.radio_c6_pending_ble_transmissions.clear();
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

    fn service_native_ble_completions(
        &mut self,
        handle: &EspC6BleBasebandHandle,
    ) -> Result<u64, MachineError> {
        let mut completed = 0_u64;
        while let Some(descriptor) = handle.take_completed_schedule() {
            let anchor = self
                .radio_c6_ble_completion_anchors
                .remove(&descriptor.address)
                .unwrap_or(descriptor.address);
            self.retire_native_ble_schedule(anchor)?;
            completed = completed.saturating_add(1);
        }
        while handle.take_acknowledged_schedule().is_some() {
            completed = completed.saturating_add(1);
        }
        Ok(completed)
    }

    fn retire_native_ble_schedule(&mut self, descriptor_address: u32) -> Result<(), MachineError> {
        // CURRENT and HEAD name the tail hardware descriptor of a linked
        // event. Same-type records reachable through word zero are the
        // earlier channel/event records retired by that one completion.
        let schedule_type =
            self.radio_read_guest_bytes(descriptor_address.wrapping_add(0x35), 1)?[0];
        let mut schedule_address = descriptor_address;
        let mut visited = Vec::new();
        // One primary-channel event has three native schedule records:
        // the current hardware tail plus the two preceding channel/list
        // records. Following beyond that reaches the next future event.
        for _ in 0..3 {
            if visited.contains(&schedule_address)
                || self.radio_read_guest_bytes(schedule_address.wrapping_add(0x35), 1)?[0]
                    != schedule_type
            {
                break;
            }
            visited.push(schedule_address);
            let linkage = self.radio_read_guest_word(schedule_address)?;
            // Bit 22 is the hardware execution-complete mark consumed by
            // the controller's task-context recycle pass.
            self.radio_write_guest_word(schedule_address, linkage | (1 << 22))?;
            let flags_address = schedule_address.wrapping_add(0x28);
            let flags = self.radio_read_guest_word(flags_address)?;
            // Bit 13 is the native baseband descriptor-load ownership flag.
            self.radio_write_guest_word(flags_address, flags & !(1 << 13))?;
            let Some(next) = c6_ble_pointer(linkage) else {
                break;
            };
            schedule_address = next;
        }
        Ok(())
    }

    fn service_native_ble_conflicts(
        &mut self,
        _handle: &EspC6BleBasebandHandle,
    ) -> Result<u64, MachineError> {
        if self.radio_c6_ble_scan.is_none() {
            return Ok(0);
        }
        let records = self.radio_c6_ble_schedule_records.clone();
        let mut skipped = 0_u64;
        for record in records {
            let schedule_type = self.radio_read_guest_bytes(record.wrapping_add(0x35), 1)?[0];
            let flags_address = record.wrapping_add(0x28);
            let flags = self.radio_read_guest_word(flags_address)?;
            if schedule_type != 1
                || flags & (1 << 13) == 0
                || self
                    .radio_c6_ble_completion_anchors
                    .values()
                    .any(|anchor| *anchor == record)
            {
                continue;
            }
            // A scan kick preempts the baseband's already-loaded advertising
            // record immediately. Hardware releases descriptor ownership (bit
            // 13) and marks the displaced entry complete (word-zero bit 22),
            // allowing task-context recycling to distinguish it from a future
            // record which remains owned by the scheduler.
            self.radio_write_guest_word(flags_address, flags & !(1 << 13))?;
            let linkage = self.radio_read_guest_word(record)?;
            self.radio_write_guest_word(record, linkage | (1 << 22))?;
            skipped = skipped.saturating_add(1);
        }
        Ok(skipped)
    }

    fn service_native_ble_schedules(
        &mut self,
        handle: &EspC6BleBasebandHandle,
    ) -> Result<u64, MachineError> {
        let mut submitted = 0_u64;
        while let Some(schedule) = handle.take_schedule() {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .validate_dma(
                    RadioSubsystem::BluetoothLe,
                    RadioDmaDirection::Transmit,
                    schedule.address,
                    4,
                    0x38,
                    0x38,
                    self.now,
                )?;
            let loaded_linkage = self.radio_read_guest_word(schedule.address)?;
            handle.set_loaded_schedule_successor(schedule.address, c6_ble_pointer(loaded_linkage));
            let schedule_type =
                self.radio_read_guest_bytes(schedule.address.wrapping_add(0x35), 1)?[0];
            let state = self.radio_read_guest_word(schedule.address.wrapping_add(4))?;
            let state_pointer = c6_ble_pointer(state);
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::MemoryMapping,
                    state_pointer.is_some(),
                    self.now,
                    format!(
                        "native schedule {:#010x} has a null controller-state pointer",
                        schedule.address
                    ),
                )?;
            let state = state_pointer.expect("legality check established controller state");
            match schedule_type {
                1 => {
                    let mut record = schedule.address;
                    let mut final_record = schedule.address;
                    let mut final_end = None;
                    for channel in [37_u8, 38, 39] {
                        let record_type =
                            self.radio_read_guest_bytes(record.wrapping_add(0x35), 1)?[0];
                        if record_type != schedule_type {
                            break;
                        }
                        let record_state = self.radio_read_guest_word(record.wrapping_add(4))?;
                        let Some(record_state) = c6_ble_pointer(record_state) else {
                            break;
                        };
                        if !self.radio_c6_ble_schedule_records.contains(&record) {
                            self.radio_c6_ble_schedule_records.push(record);
                        }
                        let Some(frame) = self.read_native_ble_advertisement(record_state)? else {
                            break;
                        };
                        let start_tick = self.radio_read_guest_word(record.wrapping_add(8))?;
                        let end_tick = self.radio_read_guest_word(record.wrapping_add(0x0c))?;
                        let start = self
                            .now
                            .checked_add(SimDuration::from_ticks(
                                handle.scheduler_delay_ticks(self.now, start_tick),
                            ))
                            .map_err(|_| MachineError::TimeOverflow)?;
                        let end = start
                            .checked_add(SimDuration::from_ticks(
                                handle.scheduler_interval_ticks(end_tick.wrapping_sub(start_tick)),
                            ))
                            .map_err(|_| MachineError::TimeOverflow)?;
                        let pending = (start, ble_advertising_spectrum(channel), frame);
                        let insertion = self
                            .radio_c6_pending_ble_transmissions
                            .iter()
                            .position(|queued| queued.0 > start)
                            .unwrap_or(self.radio_c6_pending_ble_transmissions.len());
                        self.radio_c6_pending_ble_transmissions
                            .insert(insertion, pending);
                        let flags_address = record.wrapping_add(0x28);
                        let flags = self.radio_read_guest_word(flags_address)?;
                        self.radio_write_guest_word(flags_address, flags & !(1 << 13))?;
                        final_end = Some(end);
                        final_record = record;
                        submitted = submitted.saturating_add(1);
                        let linkage = self.radio_read_guest_word(record)?;
                        let Some(next) = c6_ble_pointer(linkage) else {
                            break;
                        };
                        record = next;
                    }
                    if let Some(end) = final_end {
                        self.radio_c6_ble_completion_anchors
                            .insert(final_record, schedule.address);
                        let successor = c6_ble_pointer(self.radio_read_guest_word(final_record)?);
                        handle.schedule_successful_event_end(end, final_record, successor);
                    }
                }
                2 => {
                    let mut record = schedule.address;
                    let mut final_record = schedule.address;
                    let mut final_end = None;
                    for _ in 0..3 {
                        let record_type =
                            self.radio_read_guest_bytes(record.wrapping_add(0x35), 1)?[0];
                        if record_type != schedule_type {
                            break;
                        }
                        if !self.radio_c6_ble_schedule_records.contains(&record) {
                            self.radio_c6_ble_schedule_records.push(record);
                        }
                        let start_tick = self.radio_read_guest_word(record.wrapping_add(8))?;
                        let end_tick = self.radio_read_guest_word(record.wrapping_add(0x0c))?;
                        let start = self
                            .now
                            .checked_add(SimDuration::from_ticks(
                                handle.scheduler_delay_ticks(self.now, start_tick),
                            ))
                            .map_err(|_| MachineError::TimeOverflow)?;
                        let end = start
                            .checked_add(SimDuration::from_ticks(
                                handle.scheduler_interval_ticks(end_tick.wrapping_sub(start_tick)),
                            ))
                            .map_err(|_| MachineError::TimeOverflow)?;
                        // Scan descriptors delimit the short PHY setup edge at
                        // +8/+0c. The native baseband state separately carries
                        // the active receive-window timeout at +0x2c (for
                        // example, 9_990 us for a requested 10 ms window). The
                        // event-end interrupt belongs at the receive-window
                        // boundary, not at the setup edge; otherwise firmware
                        // restarts the scanner every few dozen microseconds and
                        // starves its host task.
                        let receive_window_ticks =
                            self.radio_read_guest_word(state.wrapping_add(0x2c))?;
                        let receive_end = if receive_window_ticks != 0
                            && receive_window_ticks < 0x0100_0000
                            && receive_window_ticks > end_tick.wrapping_sub(start_tick)
                        {
                            start
                                .checked_add(SimDuration::from_ticks(
                                    handle.scheduler_interval_ticks(receive_window_ticks),
                                ))
                                .map_err(|_| MachineError::TimeOverflow)?
                        } else {
                            end
                        };
                        final_record = record;
                        final_end = Some(receive_end);
                        let flags_address = record.wrapping_add(0x28);
                        let flags = self.radio_read_guest_word(flags_address)?;
                        self.radio_write_guest_word(flags_address, flags & !(1 << 13))?;
                        let linkage = self.radio_read_guest_word(record)?;
                        let Some(next) = c6_ble_pointer(linkage) else {
                            break;
                        };
                        record = next;
                    }
                    // CURRENT identifies the hardware tail record, while the
                    // first submitted record is only the software lifecycle
                    // anchor. RX completion must publish the same tail as a
                    // timeout completion or the ISR dispatches the wrong
                    // schedule entry and never drains the filled RX ring.
                    self.radio_c6_ble_scan = Some((final_record, state));
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .tune_receiver(Receiver {
                            node: EMULATED_NODE,
                            protocol: RadioProtocol::BluetoothLe,
                            spectrum: Spectrum::new(2_480_000, 2_000),
                            sensitivity_dbm: -100,
                        })?;
                    if let Some(end) = final_end {
                        self.radio_c6_ble_completion_anchors
                            .insert(final_record, schedule.address);
                        let successor = c6_ble_pointer(self.radio_read_guest_word(final_record)?);
                        handle.schedule_event_end(end, final_record, successor);
                    }
                    submitted = submitted.saturating_add(1);
                }
                _ => {}
            }
        }
        Ok(submitted)
    }

    fn submit_pending_native_ble_frames(&mut self) -> Result<u64, MachineError> {
        let mut submitted = 0_u64;
        while self
            .radio_c6_pending_ble_transmissions
            .first()
            .is_some_and(|pending| pending.0 <= self.now)
        {
            let (_, spectrum, bytes) = self.radio_c6_pending_ble_transmissions.remove(0);
            let duration = frame_duration(bytes.len());
            let decision = self
                .radio_coexistence
                .as_mut()
                .expect("ESP32-C6 machine has a coexistence arbiter")
                .request(CoexistenceRequest {
                    protocol: RadioProtocol::BluetoothLe,
                    start: self.now,
                    duration,
                    priority: 9,
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
                    RadioSubsystem::BluetoothLe,
                    RadioProtocol::BluetoothLe,
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
                        protocol: RadioProtocol::BluetoothLe,
                        spectrum,
                        phy: "ble-1m".to_owned(),
                        bytes,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    fn read_native_ble_advertisement(
        &mut self,
        state: u32,
    ) -> Result<Option<Vec<u8>>, MachineError> {
        // The advertising state owns a primary-channel TX-buffer pair at
        // +0x60/+0x68. Each allocation is a native memory-manager header whose
        // +8 word names the actual baseband PDU buffer. Prefer the current
        // buffer and retain the alternate slot for the controller's swap path.
        let mut tx_header = None;
        for offset in [0x60, 0x68] {
            if let Some(candidate) =
                c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(offset))?)
            {
                tx_header = Some(candidate);
                break;
            }
        }
        let Some(tx_header) = tx_header else {
            return Ok(None);
        };
        let Some(pdu_base) = c6_ble_pointer(self.radio_read_guest_word(tx_header.wrapping_add(8))?)
        else {
            return Ok(None);
        };
        let pdu = self.radio_read_guest_bytes(pdu_base.wrapping_add(0x10), 2)?;
        let payload_length = usize::from(pdu[1]);
        if payload_length < 6 || payload_length > 37 {
            return Ok(None);
        }
        let address = self.radio_read_guest_bytes(state.wrapping_add(0x34), 6)?;
        let payload = self.radio_read_guest_bytes(
            pdu_base.wrapping_add(0x12),
            payload_length.saturating_sub(6),
        )?;
        let mut frame = Vec::with_capacity(payload_length + 2);
        let random_address = address[5] & 0xc0 == 0xc0;
        frame.push(pdu[0] | if random_address { 1 << 6 } else { 0 });
        frame.push(pdu[1]);
        frame.extend_from_slice(&address);
        frame.extend_from_slice(&payload);
        Ok(Some(frame))
    }

    fn service_ble_security_dma(
        &mut self,
        handle: &EspC6BleControlHandle,
    ) -> Result<u64, MachineError> {
        let mut completed = 0_u64;
        while let Some(command) = handle.take_ecb_command() {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::DmaLength,
                    command.length == 16,
                    self.now,
                    format!(
                        "native BLE AES-ECB DMA length {} is not one 16-byte block",
                        command.length
                    ),
                )?;
            for (direction, address) in [
                (RadioDmaDirection::Transmit, command.input_address),
                (RadioDmaDirection::Receive, command.output_address),
            ] {
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .validate_dma(
                        RadioSubsystem::BluetoothLe,
                        direction,
                        address,
                        1,
                        16,
                        16,
                        self.now,
                    )?;
            }
            let mut input = [0_u8; 16];
            for (offset, byte) in input.iter_mut().enumerate() {
                *byte = self.bus.read(
                    u64::from(command.input_address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    AccessKind::Read,
                    self.now,
                )? as u8;
            }
            let output = command.encrypt_block(input);
            for (offset, byte) in output.into_iter().enumerate() {
                self.bus.write(
                    u64::from(command.output_address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(byte),
                    self.now,
                )?;
            }
            handle.complete_ecb();
            completed = completed.saturating_add(1);
        }
        Ok(completed)
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
