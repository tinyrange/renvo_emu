impl RiscVMachine {
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
            self.retire_native_ble_schedule(anchor, descriptor.address)?;
            completed = completed.saturating_add(1);
        }
        while handle.take_acknowledged_schedule().is_some() {
            completed = completed.saturating_add(1);
        }
        Ok(completed)
    }

    fn retire_native_ble_schedule(
        &mut self,
        descriptor_address: u32,
        tail_address: u32,
    ) -> Result<(), MachineError> {
        // CURRENT names the hardware tail of a linked event. Retire the exact
        // prefix from its software anchor through that tail. Extended events
        // append a different-type auxiliary record after the three primary
        // records, while the following future event must remain owned.
        let mut schedule_address = descriptor_address;
        let mut visited = Vec::new();
        let mut reached_tail = false;
        for _ in 0..16 {
            if visited.contains(&schedule_address) {
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
            if schedule_address == tail_address {
                reached_tail = true;
                break;
            }
            let Some(next) = c6_ble_pointer(linkage) else {
                break;
            };
            schedule_address = next;
        }
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::MemoryMapping,
                reached_tail,
                self.now,
                format!(
                    "native schedule {descriptor_address:#010x} does not reach completion tail {tail_address:#010x}"
                ),
            )?;
        Ok(())
    }

    fn service_native_ble_conflicts(
        &mut self,
        _handle: &EspC6BleBasebandHandle,
    ) -> Result<u64, MachineError> {
        self.radio_c6_ble_receptions
            .retain(|pending| pending.end >= self.now);
        if self.radio_c6_ble_receptions.is_empty() {
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
            let tx_header = c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(0x60))?);
            let rx_header = c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(8))?)
                .and_then(|cursor| cursor.checked_sub(4))
                .or(c6_ble_pointer(
                    self.radio_read_guest_word(state.wrapping_add(0x5c))?,
                ));
            handle.set_loaded_buffer_headers(schedule.address, tx_header, rx_header);
            match schedule_type {
                1 => {
                    let mut record = schedule.address;
                    let mut final_record = schedule.address;
                    let mut final_end = None;
                    let mut auxiliary = None;
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
                        auxiliary = auxiliary.or_else(|| ble_aux_pointer(&frame));
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
                        let spectrum = ble_advertising_spectrum(channel);
                        let response =
                            matches!(frame.first().map(|header| header & 0x0f), Some(0) | Some(6))
                                .then(|| {
                                    let frame_end = start
                                        .checked_add(frame_duration(frame.len()))
                                        .expect("validated BLE frame duration");
                                    let response_start = frame_end
                                        .checked_add(SimDuration::from_ticks(
                                            C6_BLE_INTERFRAME_SPACE_TICKS,
                                        ))
                                        .expect("validated BLE inter-frame spacing");
                                    PendingNativeBleReception {
                                        start: response_start,
                                        end,
                                        schedule_address: record,
                                        state: record_state,
                                        spectrum,
                                        rx_buffer_identifier: 0,
                                    }
                                });
                        let pending = PendingNativeBleTransmission {
                            start,
                            spectrum,
                            phy: "ble-1m",
                            bytes: frame,
                            response,
                        };
                        let insertion = self
                            .radio_c6_pending_ble_transmissions
                            .iter()
                            .position(|queued| queued.start > start)
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
                    if let Some(auxiliary) = auxiliary
                        && let Some(auxiliary_record) =
                            c6_ble_pointer(self.radio_read_guest_word(final_record)?)
                    {
                        let auxiliary_state =
                            self.radio_read_guest_word(auxiliary_record.wrapping_add(4))?;
                        if let Some(auxiliary_state) = c6_ble_pointer(auxiliary_state)
                            && let Some(frame) =
                                self.read_native_ble_auxiliary_advertisement(auxiliary_state)?
                            && frame.first().is_some_and(|header| header & 0x0f == 7)
                        {
                            if !self
                                .radio_c6_ble_schedule_records
                                .contains(&auxiliary_record)
                            {
                                self.radio_c6_ble_schedule_records.push(auxiliary_record);
                            }
                            let start_tick =
                                self.radio_read_guest_word(auxiliary_record.wrapping_add(8))?;
                            let end_tick =
                                self.radio_read_guest_word(auxiliary_record.wrapping_add(0x0c))?;
                            let start = self
                                .now
                                .checked_add(SimDuration::from_ticks(
                                    handle.scheduler_delay_ticks(self.now, start_tick),
                                ))
                                .map_err(|_| MachineError::TimeOverflow)?;
                            let end = start
                                .checked_add(SimDuration::from_ticks(
                                    handle.scheduler_interval_ticks(
                                        end_tick.wrapping_sub(start_tick),
                                    ),
                                ))
                                .map_err(|_| MachineError::TimeOverflow)?;
                            let _auxiliary_offset_ticks =
                                handle.scheduler_interval_ticks(u32::from(auxiliary.offset_us));
                            let pending = PendingNativeBleTransmission {
                                start,
                                spectrum: ble_data_spectrum(auxiliary.channel),
                                phy: auxiliary.phy,
                                bytes: frame,
                                response: None,
                            };
                            let insertion = self
                                .radio_c6_pending_ble_transmissions
                                .iter()
                                .position(|queued| queued.start > start)
                                .unwrap_or(self.radio_c6_pending_ble_transmissions.len());
                            self.radio_c6_pending_ble_transmissions
                                .insert(insertion, pending);
                            let flags_address = auxiliary_record.wrapping_add(0x28);
                            let flags = self.radio_read_guest_word(flags_address)?;
                            self.radio_write_guest_word(flags_address, flags & !(1 << 13))?;
                            final_record = auxiliary_record;
                            final_end = Some(end);
                            submitted = submitted.saturating_add(1);
                        }
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
                    if let Some(end) = final_end {
                        let activity = PendingNativeBleReception {
                            start: self.now,
                            end,
                            schedule_address: final_record,
                            state,
                            spectrum: Spectrum::new(2_480_000, 2_000),
                            rx_buffer_identifier: 5,
                        };
                        self.radio_c6_ble_receptions.push(activity);
                    }
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
                3 => {
                    // Genuine peripheral-role connection firmware writes a
                    // type-three record after accepting CONNECT_IND. Bits
                    // 8..14 of +0x14 select the PHY frequency; the remaining
                    // bits are radio configuration, not a time interval. The
                    // scheduler materializes the hardware start/end ticks at
                    // +8/+0c when it inserts the record. A peripheral listens
                    // first; it must not transmit its queued empty PDU unless
                    // a central packet actually arrives.
                    let radio_config =
                        self.radio_read_guest_word(schedule.address.wrapping_add(0x14))?;
                    let frequency_index = ((radio_config >> 8) & 0x7f) as u8;
                    let channel = ble_data_channel_from_frequency_index(frequency_index);
                    let start_tick =
                        self.radio_read_guest_word(schedule.address.wrapping_add(8))?;
                    let end_tick =
                        self.radio_read_guest_word(schedule.address.wrapping_add(0x0c))?;
                    let window_units = end_tick.wrapping_sub(start_tick);
                    let access_address = self.radio_read_guest_word(state.wrapping_add(0x30))?;
                    self.radio_legality
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio legality validator")
                        .require(
                            RadioSubsystem::BluetoothLe,
                            RadioLegalityRule::SchedulerState,
                            channel.is_some()
                                && window_units != 0
                                && window_units < 0x0100_0000
                                && access_address != 0
                                && access_address != 0x8e89_bed6,
                            self.now,
                            format!(
                                "native connection schedule is invalid: frequency_index={frequency_index} window={window_units} access_address={access_address:#010x}"
                            ),
                    )?;
                    let channel = channel.expect("legality check established BLE data channel");
                    let start = self
                        .now
                        .checked_add(SimDuration::from_ticks(
                            handle.scheduler_delay_ticks(self.now, start_tick),
                        ))
                        .map_err(|_| MachineError::TimeOverflow)?;
                    let end = start
                        .checked_add(SimDuration::from_ticks(
                            handle.scheduler_interval_ticks(window_units),
                        ))
                        .map_err(|_| MachineError::TimeOverflow)?;
                    let spectrum = ble_data_spectrum(channel);
                    self.radio_c6_ble_link_sequences.entry(state).or_default();
                    let successor = c6_ble_pointer(self.radio_read_guest_word(schedule.address)?);
                    self.radio_medium
                        .as_mut()
                        .expect("ESP32-C6 machine has a radio medium")
                        .tune_receiver(Receiver {
                            node: EMULATED_NODE,
                            protocol: RadioProtocol::BluetoothLe,
                            spectrum,
                            sensitivity_dbm: -100,
                        })?;
                    self.radio_c6_ble_receptions
                        .push(PendingNativeBleReception {
                            start,
                            end,
                            schedule_address: schedule.address,
                            state,
                            spectrum,
                            // r_ble_lll_conn_recycle_buffer maps the native RX
                            // identifier to a connection index by subtracting
                            // one plus pinned controller config byte 0x42. That
                            // byte is one for this firmware, so connection zero
                            // owns identifier two.
                            rx_buffer_identifier: 2,
                        });
                    self.radio_c6_ble_completion_anchors
                        .insert(schedule.address, schedule.address);
                    handle.schedule_event_end(end, schedule.address, successor);
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
            .is_some_and(|pending| pending.start <= self.now)
        {
            let pending = self.radio_c6_pending_ble_transmissions.remove(0);
            let PendingNativeBleTransmission {
                spectrum,
                phy,
                bytes,
                response,
                ..
            } = pending;
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
                        phy: phy.to_owned(),
                        bytes,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
            self.record_coexistence_transmission(grant, transmission);
            if let Some(response) = response {
                self.radio_medium
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio medium")
                    .tune_receiver(Receiver {
                        node: EMULATED_NODE,
                        protocol: RadioProtocol::BluetoothLe,
                        spectrum: response.spectrum,
                        sensitivity_dbm: -100,
                    })?;
                self.radio_c6_ble_receptions.push(response);
            }
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
        self.read_native_ble_pdu(state, tx_header, true)
    }

    fn read_native_ble_auxiliary_advertisement(
        &mut self,
        state: u32,
    ) -> Result<Option<Vec<u8>>, MachineError> {
        // The schedule retains the native secondary-channel owner directly.
        // Its +0x60 TX-list head is also the primary PDU allocation; the
        // compressed +4 link names the auxiliary allocation's +4 word. Recover
        // that allocation header before reading its +8 PDU pointer.
        let tx_list_raw = self.radio_read_guest_word(state.wrapping_add(0x60))?;
        let Some(tx_list) = c6_ble_pointer(tx_list_raw) else {
            return Ok(None);
        };
        let tx_link_raw = self.radio_read_guest_word(tx_list.wrapping_add(4))?;
        let Some(tx_link) = c6_ble_pointer(tx_link_raw) else {
            return Ok(None);
        };
        let Some(tx_header) = tx_link.checked_sub(4) else {
            return Ok(None);
        };
        self.read_native_ble_pdu(state, tx_header, false)
    }

    fn read_native_ble_pdu(
        &mut self,
        state: u32,
        tx_header: u32,
        reconstruct_legacy_address: bool,
    ) -> Result<Option<Vec<u8>>, MachineError> {
        let Some(pdu_base) = c6_ble_pointer(self.radio_read_guest_word(tx_header.wrapping_add(8))?)
        else {
            return Ok(None);
        };
        let pdu = self.radio_read_guest_bytes(pdu_base.wrapping_add(0x10), 2)?;
        let payload_length = usize::from(pdu[1]);
        let pdu_type = pdu[0] & 0x0f;
        if pdu_type == 7 {
            let payload =
                self.radio_read_guest_bytes(pdu_base.wrapping_add(0x12), payload_length)?;
            let mut frame = Vec::with_capacity(payload_length + 2);
            let inserts_advertiser_address = payload
                .get(1)
                .is_some_and(|extended_header_flags| extended_header_flags & 1 != 0);
            if inserts_advertiser_address && payload_length >= 8 {
                // C6 hardware inserts AdvA after the extended-header flags.
                // The controller stores the following fields and AdvData
                // contiguously, leaving six reserved bytes at the native PDU
                // tail so its programmed on-air length already includes AdvA.
                let address = self.radio_read_guest_bytes(state.wrapping_add(0x34), 6)?;
                let random_address = address[5] & 0xc0 == 0xc0;
                frame.push(pdu[0] | if random_address { 1 << 6 } else { 0 });
                frame.push(pdu[1]);
                frame.extend_from_slice(&payload[..2]);
                frame.extend_from_slice(&address);
                frame.extend_from_slice(&payload[2..payload_length - 6]);
            } else {
                frame.extend_from_slice(&pdu);
                frame.extend_from_slice(&payload);
            }
            return Ok(Some(frame));
        }
        if !reconstruct_legacy_address {
            let payload =
                self.radio_read_guest_bytes(pdu_base.wrapping_add(0x12), payload_length)?;
            let mut frame = Vec::with_capacity(payload_length + 2);
            frame.extend_from_slice(&pdu);
            frame.extend_from_slice(&payload);
            return Ok(Some(frame));
        }
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

    fn native_ble_connection_tx_header(&mut self, state: u32) -> Result<Option<u32>, MachineError> {
        // Connection state uses +0x60 as a sentinel list head. Unlike the
        // advertising state, the sentinel is not itself a TX allocation: its
        // compressed +4 link points four bytes into the first allocation.
        // The hardware cursor and over-air decoder both operate on that first
        // real allocation header.
        let Some(list_head) = c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(0x60))?)
        else {
            return Ok(None);
        };
        Ok(
            c6_ble_pointer(self.radio_read_guest_word(list_head.wrapping_add(4))?)
                .and_then(|link| link.checked_sub(4)),
        )
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

}
