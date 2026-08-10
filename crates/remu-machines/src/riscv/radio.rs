use super::{MachineError, RiscVMachine};
use crate::TargetId;
use remu_core::{AccessKind, AccessWidth, Bus, SimDuration};
use remu_devices::{
    EspC6BleBasebandHandle, EspC6BleControlHandle, EspIeee802154Command, EspIeee802154Handle,
};
use remu_radio::{
    BleController, CoexistenceDecision, CoexistenceRequest, DeliveryOutcome, ExtendedAddress,
    FrameOrigin, Ieee802154CcaMode, Ieee802154Error, Ieee802154Mac, Ieee802154RxOutcome,
    MediumEvent, NodeId, PanInterface, RadioFrame, RadioProtocol, Receiver, ReplayArtifact,
    ShortAddress, Spectrum, TxRequest, WifiEngine,
};

const EMULATED_NODE: NodeId = NodeId(1);
const HOST_NODE: NodeId = NodeId(0);

impl RiscVMachine {
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
        ieee802154.poll(self.now);
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
                    } else {
                        ieee802154.abort(false, 24);
                    }
                }
                EspIeee802154Command::Stop | EspIeee802154Command::TestStop => {
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
        interrupt_matrix.set_source(
            0,
            wifi_mac.interrupt_pending()
                || self.radio_wifi.as_ref().is_some_and(WifiEngine::has_rx),
        );
        interrupt_matrix.set_source(
            4,
            ble_baseband.interrupt_pending()
                || self
                    .radio_ble
                    .as_ref()
                    .is_some_and(BleController::has_h4_output),
        );
        // Source 5 is the separately exposed BT_BB line. Current C6 controller
        // firmware installs its combined native PHY ISR on BT_MAC source 4,
        // while freestanding stacks may route the baseband source directly.
        interrupt_matrix.set_source(5, ble_baseband.interrupt_pending());
        interrupt_matrix.set_source(12, ieee802154.interrupt_pending());
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
            let loaded_linkage = self.radio_read_guest_word(schedule.address)?;
            handle.set_loaded_schedule_successor(schedule.address, c6_ble_pointer(loaded_linkage));
            let schedule_type =
                self.radio_read_guest_bytes(schedule.address.wrapping_add(0x35), 1)?[0];
            let state = self.radio_read_guest_word(schedule.address.wrapping_add(4))?;
            let Some(state) = c6_ble_pointer(state) else {
                continue;
            };
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
                        self.radio_medium
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio medium")
                            .transmit(TxRequest {
                                source: EMULATED_NODE,
                                start,
                                end,
                                power_dbm: 0,
                                frame: RadioFrame {
                                    protocol: RadioProtocol::BluetoothLe,
                                    spectrum: ble_advertising_spectrum(channel),
                                    phy: "ble-1m".to_owned(),
                                    bytes: frame,
                                    origin: FrameOrigin::Emulated,
                                },
                            })?;
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
            if command.length != 16 {
                handle.complete_ecb();
                continue;
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
            let Ok(buffer) = self.bus.read(
                u64::from(descriptor.address.wrapping_add(4)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            ) else {
                continue;
            };
            let buffer = buffer as u32;
            let Ok(length) = self.bus.read(
                u64::from(buffer),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            ) else {
                continue;
            };
            let length = length as usize;
            if length == 0 || length > 4095 {
                continue;
            }
            let Ok(bytes) = self.radio_read_guest_bytes(buffer.wrapping_add(8), length) else {
                continue;
            };
            let duration = frame_duration(bytes.len());
            self.radio_medium
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
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    fn complete_ieee802154_cca_tx(
        &mut self,
        modem: &remu_devices::EspC6ModemHandle,
        handle: &EspIeee802154Handle,
    ) -> Result<(), MachineError> {
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
        let CoexistenceDecision::Granted { id: grant, .. } = decision else {
            handle.abort(true, 18);
            return Ok(());
        };
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
        self.radio_pending_ieee802154_tx
            .push((id, grant, end, ack_sequence));
        Ok(())
    }

    fn complete_medium_receptions(
        &mut self,
        handle: &EspIeee802154Handle,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
        ble_baseband: &EspC6BleBasebandHandle,
    ) -> Result<u64, MachineError> {
        let medium = self
            .radio_medium
            .as_ref()
            .expect("ESP32-C6 machine has a radio medium");
        let new_events = &medium.events()[self.radio_event_cursor..];
        let mut deliveries = Vec::new();
        for event in new_events {
            let MediumEvent::Reception {
                id,
                receiver: EMULATED_NODE,
                outcome,
            } = event
            else {
                continue;
            };
            let transmission = medium
                .events()
                .iter()
                .find_map(|candidate| match candidate {
                    MediumEvent::Submitted {
                        id: candidate_id,
                        request,
                    } if candidate_id == id => Some((
                        request.frame.clone(),
                        medium.received_power_dbm(request.source, EMULATED_NODE, request.power_dbm),
                    )),
                    _ => None,
                });
            deliveries.push((outcome.clone(), transmission));
        }
        self.radio_event_cursor = medium.events().len();
        let mut completed = 0_u64;
        for (outcome, transmission) in deliveries {
            match (outcome, transmission) {
                (DeliveryOutcome::Delivered, Some((frame, received_power_dbm)))
                    if frame.protocol == RadioProtocol::Ieee802154 && handle.receiving() =>
                {
                    if let Some(sequence) = handle.awaiting_ack_sequence() {
                        if frame.bytes.len() >= 5
                            && Ieee802154Mac::has_valid_fcs(&frame.bytes)
                            && frame.bytes[0] & 7 == 2
                            && frame.bytes[2] == sequence
                        {
                            self.write_ieee802154_ack_rx(
                                handle,
                                &frame.bytes[..frame.bytes.len() - 2],
                                received_power_dbm,
                            )?;
                        } else {
                            handle.abort(true, 8);
                        }
                        completed = completed.saturating_add(1);
                        continue;
                    }
                    let outcome = self
                        .radio_ieee802154_mac
                        .as_mut()
                        .expect("ESP32-C6 machine has an IEEE 802.15.4 MAC")
                        .receive(&frame.bytes);
                    match outcome {
                        Ok(Ieee802154RxOutcome::Accepted { frame, .. }) => {
                            self.write_ieee802154_rx(handle, &frame, received_power_dbm)?;
                            completed = completed.saturating_add(1);
                        }
                        Ok(Ieee802154RxOutcome::AcceptedWithAck { frame, ack, .. }) => {
                            self.write_ieee802154_rx(handle, &frame, received_power_dbm)?;
                            self.submit_ieee802154_ack(handle, ack)?;
                            completed = completed.saturating_add(1);
                        }
                        Ok(Ieee802154RxOutcome::Filtered) => {
                            handle.record_filter_failure();
                            completed = completed.saturating_add(1);
                        }
                        Err(remu_radio::Ieee802154Error::InvalidFcs) => {
                            handle.abort(false, 3);
                            completed = completed.saturating_add(1);
                        }
                        Err(_) => {
                            handle.abort(false, 4);
                            completed = completed.saturating_add(1);
                        }
                    }
                }
                (DeliveryOutcome::Delivered, Some((frame, _)))
                    if frame.protocol == RadioProtocol::Wifi =>
                {
                    let native = self.write_native_wifi_rx(wifi_mac, &frame.bytes)?;
                    if self
                        .esp32c6_peripherals
                        .as_ref()
                        .is_some_and(|handles| handles.modem.wifi_ready())
                        && self
                            .radio_wifi
                            .as_mut()
                            .expect("ESP32-C6 has a Wi-Fi engine")
                            .receive(&frame)
                            .unwrap_or(false)
                    {
                        completed = completed.saturating_add(1);
                    } else if native {
                        completed = completed.saturating_add(1);
                    }
                }
                (DeliveryOutcome::Delivered, Some((frame, signal_dbm)))
                    if frame.protocol == RadioProtocol::BluetoothLe =>
                {
                    let native =
                        self.write_native_ble_rx(ble_baseband, &frame.bytes, signal_dbm)?;
                    let protocol_engine = self
                        .esp32c6_peripherals
                        .as_ref()
                        .is_some_and(|handles| handles.modem.ble_ready())
                        && self
                            .radio_ble
                            .as_mut()
                            .expect("ESP32-C6 has a BLE controller")
                            .receive_rf(&frame, signal_dbm.clamp(-128, 127) as i8)
                            .unwrap_or(false);
                    if native || protocol_engine {
                        completed = completed.saturating_add(1);
                    }
                }
                (DeliveryOutcome::Collision { .. } | DeliveryOutcome::SeededLoss, _) => {
                    handle.abort(false, 3);
                    completed = completed.saturating_add(1);
                }
                (DeliveryOutcome::BelowSensitivity { .. }, _) | (_, None) => {}
                (DeliveryOutcome::Delivered, Some(_)) => {}
            }
        }
        Ok(completed)
    }

    fn write_native_ble_rx(
        &mut self,
        handle: &EspC6BleBasebandHandle,
        frame: &[u8],
        signal_dbm: i16,
    ) -> Result<bool, MachineError> {
        let Some((schedule_address, state)) = self.radio_c6_ble_scan else {
            return Ok(false);
        };
        if frame.len() < 2 || frame.len() > u8::MAX as usize {
            return Ok(false);
        }

        let Some(mut header) =
            c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(0x5c))?)
        else {
            return Ok(false);
        };
        let mut selected = None;
        for _ in 0..64 {
            if let Some(buffer) =
                c6_ble_pointer(self.radio_read_guest_word(header.wrapping_add(8))?)
                && self.radio_read_guest_word(buffer.wrapping_add(0x18))? & 0xffff == 0xffff
            {
                selected = Some(buffer);
                break;
            }
            let Some(next_plus_four) =
                c6_ble_pointer(self.radio_read_guest_word(header.wrapping_add(4))?)
            else {
                break;
            };
            let next = next_plus_four.wrapping_sub(4);
            if next == header {
                break;
            }
            header = next;
        }
        let Some(buffer) = selected else {
            return Ok(false);
        };

        // Native RX buffers contain sixteen bytes of hardware metadata before
        // the over-air PDU. The status word carries signed RSSI in its high
        // byte; bit 10 is the CRC-error indication and remains clear for a
        // successfully delivered medium frame. RX-info's low half is the native RX
        // header span (eight bytes for this legacy advertising path), followed
        // by the 2402-MHz-relative frequency index in bits 16..22 and PHY rate
        // in bits 24..25. The over-air length remains in the PDU header itself.
        let mut metadata = [0_u8; 16];
        let status = u32::from((signal_dbm.clamp(-128, 127) as i8) as u8) << 24;
        metadata[0..4].copy_from_slice(&status.to_le_bytes());
        metadata[4..8].copy_from_slice(&((self.now.ticks() / 16) as u32).to_le_bytes());
        metadata[12..14].copy_from_slice(&8_u16.to_le_bytes());
        metadata[14] = 78;
        metadata[15] = 0;
        self.radio_write_guest_bytes(buffer.wrapping_add(0x0c), &metadata)?;
        self.radio_write_guest_bytes(buffer.wrapping_add(0x1c), frame)?;

        // The baseband cursor names the next hardware-owned RX header. Move it
        // past the header just filled so that descriptor becomes part of the
        // completed prefix consumed by the controller's recycle walk. Header
        // link words already contain the next header's address plus four.
        // Bit 27 records that RX DMA completed during this schedule.
        let current_rx = self.radio_read_guest_word(state.wrapping_add(8))?;
        let next_rx = self.radio_read_guest_word(header.wrapping_add(4))? & 0x000f_ffff;
        if next_rx == 0 {
            return Ok(false);
        }
        self.radio_write_guest_word(state.wrapping_add(8), (current_rx & !0x000f_ffff) | next_rx)?;
        let schedule_flags = self.radio_read_guest_word(state.wrapping_add(0x14))?;
        self.radio_write_guest_word(state.wrapping_add(0x14), schedule_flags | (1 << 27))?;

        self.radio_c6_ble_scan = None;
        let successor = c6_ble_pointer(self.radio_read_guest_word(schedule_address)?);
        handle.schedule_received_event_end(self.now, schedule_address, successor);
        Ok(true)
    }

    fn write_native_wifi_rx(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
        frame: &[u8],
    ) -> Result<bool, MachineError> {
        let Some(descriptor) = wifi_mac.rx_descriptor() else {
            return Ok(false);
        };
        let control = self.bus.read(
            u64::from(descriptor.address),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        )? as u32;
        if control & (1 << 31) == 0 {
            return Ok(false);
        }
        let buffer = self.bus.read(
            u64::from(descriptor.address.wrapping_add(4)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        )? as u32;
        let next = self.bus.read(
            u64::from(descriptor.address.wrapping_add(8)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        )? as u32;
        let capacity = (control & 0x3fff) as usize;
        let metadata = c6_wifi_rx_metadata(frame.len(), self.now);
        let total = metadata.len().saturating_add(frame.len()).saturating_add(4);
        if buffer == 0 || total > capacity || total > 0x3fff {
            return Ok(false);
        }
        self.radio_write_guest_bytes(buffer, &metadata)?;
        self.radio_write_guest_bytes(buffer.wrapping_add(metadata.len() as u32), frame)?;
        self.radio_write_guest_bytes(
            buffer.wrapping_add((metadata.len() + frame.len()) as u32),
            &[0; 4],
        )?;
        let completed = (control & 0x0000_3fff) | ((total as u32) << 14) | (1 << 30);
        self.radio_write_guest_word(descriptor.address, completed)?;
        wifi_mac.complete_rx_descriptor(descriptor.address, next);
        Ok(true)
    }

    fn submit_protocol_engine_frames(&mut self) -> Result<u64, MachineError> {
        let mut frames = Vec::new();
        while let Some((_, frame)) = self.radio_wifi.as_mut().and_then(WifiEngine::take_tx) {
            frames.push((frame, 8));
        }
        while let Some(frame) = self
            .radio_ble
            .as_mut()
            .and_then(BleController::take_rf_output)
        {
            frames.push((frame, 9));
        }
        let mut submitted = 0_u64;
        for (frame, priority) in frames {
            let duration = frame_duration(frame.bytes.len());
            let decision = self
                .radio_coexistence
                .as_mut()
                .expect("ESP32-C6 has a coexistence arbiter")
                .request(CoexistenceRequest {
                    protocol: frame.protocol,
                    start: self.now,
                    duration,
                    priority,
                    preemptible: true,
                })?;
            if !matches!(decision, CoexistenceDecision::Granted { .. }) {
                continue;
            }
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 has a radio medium")
                .transmit(TxRequest {
                    source: EMULATED_NODE,
                    start: self.now,
                    end: self
                        .now
                        .checked_add(duration)
                        .map_err(|_| MachineError::TimeOverflow)?,
                    power_dbm: 0,
                    frame,
                })?;
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    fn sync_ieee802154_configuration(
        &mut self,
        handle: &EspIeee802154Handle,
    ) -> Result<(), MachineError> {
        let configuration = handle.configuration();
        let mac = self
            .radio_ieee802154_mac
            .as_mut()
            .expect("ESP32-C6 machine has an IEEE 802.15.4 MAC");
        for (index, pan) in configuration.pans.into_iter().enumerate() {
            mac.set_interface(
                u8::try_from(index).expect("four PAN slots fit in u8"),
                pan.map(|pan| PanInterface {
                    pan_id: pan.pan_id,
                    short_address: ShortAddress(pan.short_address),
                    extended_address: ExtendedAddress(pan.extended_address),
                }),
            )
            .expect("hardware exposes exactly four PAN slots");
        }
        mac.set_promiscuous(configuration.promiscuous);
        mac.set_auto_ack(configuration.automatic_ack_transmit);
        mac.set_frame_pending(configuration.frame_pending);
        mac.set_cca_threshold_dbm(i16::from(configuration.cca_threshold_dbm));
        Ok(())
    }

    fn write_ieee802154_rx(
        &mut self,
        handle: &EspIeee802154Handle,
        frame: &[u8],
        received_power_dbm: i16,
    ) -> Result<(), MachineError> {
        let (_, rx_address) = handle.dma_addresses();
        let length = frame.len().min(125);
        // The native DMA buffer retains the PSDU length including its FCS,
        // but hardware validates and replaces those final two FCS bytes with
        // RSSI and LQI. ESP-IDF consequently reads RSSI at `length - 1` and
        // LQI at `length`, after the leading PHY-length byte.
        let psdu_length = length + 2;
        self.write_guest_byte(rx_address, psdu_length as u8)?;
        for (offset, byte) in frame.iter().take(length).enumerate() {
            self.write_guest_byte(rx_address.wrapping_add(1 + offset as u32), *byte)?;
        }
        self.write_guest_byte(
            rx_address.wrapping_add(1 + length as u32),
            received_power_dbm.clamp(-128, 127) as i8 as u8,
        )?;
        self.write_guest_byte(
            rx_address.wrapping_add(2 + length as u32),
            ieee802154_lqi(received_power_dbm),
        )?;
        handle.complete_rx(psdu_length as u8);
        Ok(())
    }

    fn write_ieee802154_ack_rx(
        &mut self,
        handle: &EspIeee802154Handle,
        frame: &[u8],
        received_power_dbm: i16,
    ) -> Result<(), MachineError> {
        let (_, rx_address) = handle.dma_addresses();
        let length = frame.len().min(125);
        let psdu_length = length + 2;
        self.write_guest_byte(rx_address, psdu_length as u8)?;
        for (offset, byte) in frame.iter().take(length).enumerate() {
            self.write_guest_byte(rx_address.wrapping_add(1 + offset as u32), *byte)?;
        }
        self.write_guest_byte(
            rx_address.wrapping_add(1 + length as u32),
            received_power_dbm.clamp(-128, 127) as i8 as u8,
        )?;
        self.write_guest_byte(
            rx_address.wrapping_add(2 + length as u32),
            ieee802154_lqi(received_power_dbm),
        )?;
        handle.complete_ack_rx(psdu_length as u8);
        Ok(())
    }

    fn submit_ieee802154_ack(
        &mut self,
        handle: &EspIeee802154Handle,
        ack: Vec<u8>,
    ) -> Result<(), MachineError> {
        let spectrum = ieee802154_spectrum(handle.channel());
        let end = self
            .now
            .checked_add(frame_duration(ack.len()))
            .map_err(|_| MachineError::TimeOverflow)?;
        self.radio_medium
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
                    bytes: ack,
                    origin: FrameOrigin::Emulated,
                },
            })?;
        self.radio_pending_ieee802154_ack.push(end);
        Ok(())
    }

    fn radio_read_guest_bytes(
        &mut self,
        address: u32,
        length: usize,
    ) -> Result<Vec<u8>, MachineError> {
        (0..length)
            .map(|offset| {
                self.bus
                    .read(
                        u64::from(address.wrapping_add(offset as u32)),
                        AccessWidth::Byte,
                        AccessKind::Read,
                        self.now,
                    )
                    .map(|value| value as u8)
                    .map_err(MachineError::Bus)
            })
            .collect()
    }

    fn radio_read_guest_word(&mut self, address: u32) -> Result<u32, MachineError> {
        self.bus
            .read(
                u64::from(address),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )
            .map(|value| value as u32)
            .map_err(MachineError::Bus)
    }

    fn write_guest_byte(&mut self, address: u32, byte: u8) -> Result<(), MachineError> {
        self.bus
            .write(
                u64::from(address),
                AccessWidth::Byte,
                u64::from(byte),
                self.now,
            )
            .map_err(MachineError::Bus)
    }

    fn radio_write_guest_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<(), MachineError> {
        for (offset, byte) in bytes.iter().enumerate() {
            self.write_guest_byte(address.wrapping_add(offset as u32), *byte)?;
        }
        Ok(())
    }

    fn radio_write_guest_word(&mut self, address: u32, word: u32) -> Result<(), MachineError> {
        self.bus
            .write(
                u64::from(address),
                AccessWidth::Word,
                u64::from(word),
                self.now,
            )
            .map_err(MachineError::Bus)
    }
}

fn c6_ble_pointer(raw: u32) -> Option<u32> {
    let low = raw & 0x000f_ffff;
    (low != 0).then_some(0x4080_0000 | low)
}

fn c6_wifi_rx_metadata(frame_length: usize, at: remu_core::SimTime) -> [u8; 92] {
    let mut metadata = [0_u8; 92];
    metadata[0] = (-40_i8) as u8;
    metadata[3] = 1 << 4;
    metadata[11] = 1 << 7;
    metadata[12..16].copy_from_slice(&(at.ticks() as u32).to_le_bytes());
    metadata[20] = (-95_i8) as u8;
    metadata[21] = 1;
    let signal_length = frame_length.saturating_add(4).min(0x3fff) as u32;
    let dump_length = frame_length.min(0x3fff) as u32;
    metadata[84..88].copy_from_slice(&(signal_length | (dump_length << 16)).to_le_bytes());
    metadata
}

fn ieee802154_spectrum(channel: u8) -> Spectrum {
    Spectrum::new(2_405_000 + u32::from(channel - 11) * 5_000, 2_000)
}

fn ieee802154_lqi(received_power_dbm: i16) -> u8 {
    // Deterministic monotonic link quality over the useful 2.4 GHz receiver
    // range. Valid-FCS frames at or above -20 dBm saturate, while sensitivity
    // at -100 dBm maps to zero.
    let above_sensitivity = received_power_dbm.clamp(-100, -20) + 100;
    ((above_sensitivity * 255) / 80) as u8
}

fn ble_advertising_spectrum(channel: u8) -> Spectrum {
    let center_khz = match channel {
        37 => 2_402_000,
        38 => 2_426_000,
        _ => 2_480_000,
    };
    Spectrum::new(center_khz, 2_000)
}

fn frame_duration(length: usize) -> SimDuration {
    SimDuration::from_ticks(
        u64::try_from(length.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(32),
    )
}

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
