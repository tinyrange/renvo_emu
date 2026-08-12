impl RiscVMachine {
    pub(super) fn mark_native_ble_connection_rx_success(
        &mut self,
        schedule_address: u32,
        continuation: bool,
    ) -> Result<(), MachineError> {
        const NATIVE_SCHEDULE_RX_SUCCESS: u32 = 1 << 11;
        const NATIVE_SCHEDULE_OWNED: u32 = 1 << 13;

        let flags_address = schedule_address.wrapping_add(0x28);
        let flags = self.radio_read_guest_word(flags_address)?;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                flags & NATIVE_SCHEDULE_OWNED != 0,
                self.now,
                format!(
                    "native connection RX completed after firmware released schedule {schedule_address:#010x}: flags={flags:#010x}"
                ),
            )?;
        let already_successful = flags & NATIVE_SCHEDULE_RX_SUCCESS != 0;
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                already_successful == continuation,
                self.now,
                if continuation {
                    format!(
                        "native connection schedule {schedule_address:#010x} continued without an initial RX completion: flags={flags:#010x}"
                    )
                } else {
                    format!(
                        "native connection schedule {schedule_address:#010x} completed an unexplained duplicate RX: flags={flags:#010x}"
                    )
                },
            )?;
        if continuation {
            return Ok(());
        }
        // The pinned controller's RX interrupt consumes the completed buffer
        // before r_ble_lll_conn_recycle_sch_item runs in task context. Native
        // hardware leaves bit eleven in the schedule result word to
        // distinguish that successful radio operation from a no-packet event.
        // r_ble_lll_conn_recycle_sch_item explicitly extracts `(result >> 11)
        // & 1` and uses it to promote link state 1 (establishing) to state 2
        // (connected), after which its genuine supervision callback reports
        // reason 0x08 instead of connection-establishment failure 0x3e.
        self.radio_write_guest_word(flags_address, flags | NATIVE_SCHEDULE_RX_SUCCESS)
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
                        request.start,
                    )),
                    _ => None,
                });
            deliveries.push((outcome.clone(), transmission));
        }
        self.radio_event_cursor = medium.events().len();
        let mut completed = 0_u64;
        for (outcome, transmission) in deliveries {
            match (outcome, transmission) {
                (DeliveryOutcome::Delivered, Some((frame, received_power_dbm, _)))
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
                        self.radio_legality
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio legality validator")
                            .transition_activity(
                                RadioSubsystem::Ieee802154,
                                RadioActivity::AwaitingAck,
                                RadioActivity::Idle,
                                self.now,
                            )?;
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
                (DeliveryOutcome::Delivered, Some((frame, _, received_at)))
                    if frame.protocol == RadioProtocol::Wifi =>
                {
                    if let Some(index) = self
                        .radio_pending_native_wifi
                        .iter()
                        .position(|pending| pending.accepts_response(&frame.bytes, received_at))
                    {
                        let pending = self.radio_pending_native_wifi.remove(index);
                        self.radio_legality
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio legality validator")
                            .require(
                                RadioSubsystem::Wifi,
                                RadioLegalityRule::CompletionWithoutOperation,
                                wifi_mac.tx_active(pending.queue),
                                self.now,
                                format!(
                                    "control response arrived for inactive native TX queue {}",
                                    pending.queue
                                ),
                            )?;
                        self.radio_legality
                            .as_mut()
                            .expect("ESP32-C6 machine has a radio legality validator")
                            .require(
                                RadioSubsystem::Wifi,
                                RadioLegalityRule::CompletionWithoutOperation,
                                wifi_mac.complete_tx(
                                    pending.queue,
                                    remu_devices::EspWifiTxOutcome::Success,
                                ),
                                self.now,
                                format!(
                                    "native TX queue {} rejected its control-response completion",
                                    pending.queue
                                ),
                            )?;
                        completed = completed.saturating_add(1);
                        continue;
                    }
                    let responded = self.submit_native_wifi_receive_response(
                        wifi_mac,
                        &frame.bytes,
                    )?;
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
                    } else if native || responded {
                        completed = completed.saturating_add(1);
                    }
                }
                (DeliveryOutcome::Delivered, Some((frame, signal_dbm, _)))
                    if frame.protocol == RadioProtocol::BluetoothLe =>
                {
                    let native = self.write_native_ble_rx(ble_baseband, &frame, signal_dbm)?;
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
                (
                    DeliveryOutcome::Collision { .. } | DeliveryOutcome::SeededLoss,
                    Some((frame, _, _)),
                ) if frame.protocol == RadioProtocol::Ieee802154 => {
                    let awaiting_ack = handle.awaiting_ack_sequence().is_some();
                    handle.abort(false, 3);
                    if awaiting_ack {
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
                    completed = completed.saturating_add(1);
                }
                (DeliveryOutcome::Collision { .. } | DeliveryOutcome::SeededLoss, _) => {}
                (DeliveryOutcome::BelowSensitivity { .. }, _) | (_, None) => {}
                (DeliveryOutcome::Delivered, Some(_)) => {}
            }
        }
        Ok(completed)
    }

    fn write_native_ble_rx(
        &mut self,
        handle: &EspC6BleBasebandHandle,
        frame: &RadioFrame,
        signal_dbm: i16,
    ) -> Result<bool, MachineError> {
        self.radio_c6_ble_receptions
            .retain(|pending| pending.end >= self.now);
        let Some((activity_index, activity)) = self
            .radio_c6_ble_receptions
            .iter()
            .enumerate()
            .find(|(_, pending)| {
                pending.start <= self.now
                    && pending.end >= self.now
                    && pending.spectrum.overlaps(frame.spectrum)
                    && pending.phy == frame.phy
            })
            .map(|(index, pending)| (index, pending.clone()))
        else {
            return Ok(false);
        };
        let schedule_address = activity.schedule_address;
        let state = activity.state;
        let rx_buffer_identifier = activity.rx_buffer_identifier;
        let over_air_frame = frame.bytes.clone();
        let frame = over_air_frame.as_slice();
        if frame.len() < 2 || frame.len() > u8::MAX as usize {
            return Ok(false);
        }

        let current_rx = self.radio_read_guest_word(state.wrapping_add(8))?;
        let ring_head =
            c6_ble_pointer(self.radio_read_guest_word(state.wrapping_add(0x5c))?);
        // The native cursor contains the current RX-header address plus four,
        // matching the link word in each header. Starting every search at the
        // ring head works for the first legacy event but eventually assigns a
        // primary-channel PDU to an auxiliary-data slot during extended scan.
        let Some(mut header) = c6_ble_pointer(current_rx)
            .map(|address| address.wrapping_sub(4))
            .or(ring_head)
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

        // Once the peripheral has emitted LL_START_ENC_REQ, C6 hardware sends
        // the following encrypted LL_START_ENC_RSP through modem-security CCM.
        // Keep ciphertext in RX DMA so the genuine controller programs that
        // engine and publishes the plaintext itself.
        let hardware_encryption_transition = rx_buffer_identifier == 2
            && self
                .radio_c6_ble_link_sequences
                .entry(state)
                .or_default()
                .expects_central_response();
        let dma_frame_result = if rx_buffer_identifier == 2 {
            self.radio_c6_ble_link_sequences
                .entry(state)
                .or_default()
                .native_rx_dma_frame(frame)
        } else {
            Ok(frame.to_vec())
        };
        let (dma_frame, dma_frame_error) = match dma_frame_result {
            Ok(decoded) => (decoded, None),
            Err(detail) => (Vec::new(), Some(detail)),
        };
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                dma_frame_error.is_none(),
                self.now,
                dma_frame_error.unwrap_or_default(),
            )?;
        let hardware_empty_result = if rx_buffer_identifier == 2 {
            self
                .radio_c6_ble_link_sequences
                .entry(state)
                .or_default()
                .hardware_filters_empty(frame)
        } else {
            Ok(false)
        };
        let (hardware_empty_pdu, hardware_empty_error) = match hardware_empty_result {
            Ok(empty) => (empty, None),
            Err(detail) => (false, Some(detail)),
        };
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                hardware_empty_error.is_none(),
                self.now,
                hardware_empty_error.unwrap_or_default(),
            )?;

        // Native RX buffers contain sixteen bytes of hardware metadata before
        // the over-air PDU. The status word carries signed RSSI in its high
        // byte; bit 10 is the CRC-error indication and remains clear for a
        // successfully delivered medium frame. RX-info's low half is the native RX
        // controller RX-buffer identifier (five is the first scanner-owned
        // slot in the pinned BLE 5 controller configuration), followed
        // by the 2402-MHz-relative frequency index in bits 16..22 and PHY rate
        // in bits 24..25. The over-air length remains in the PDU header itself.
        let mut metadata = [0_u8; 16];
        let status = u32::from((signal_dbm.clamp(-128, 127) as i8) as u8) << 24;
        metadata[0..4].copy_from_slice(&status.to_le_bytes());
        metadata[4..8].copy_from_slice(&handle.scheduler_timestamp(self.now).to_le_bytes());
        metadata[12..14].copy_from_slice(&rx_buffer_identifier.to_le_bytes());
        metadata[14] = 78;
        metadata[15] = 0;
        let native_rx_counter = if rx_buffer_identifier == 2 && !hardware_empty_pdu {
            self.radio_c6_ble_link_sequences
                .entry(state)
                .or_default()
                .native_rx_packet_counter(frame)
        } else {
            Ok(None)
        };
        let (native_rx_counter, counter_error) = match native_rx_counter {
            Ok(counter) => (counter, None),
            Err(detail) => (None, Some(detail)),
        };
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                counter_error.is_none(),
                self.now,
                counter_error.unwrap_or_default(),
            )?;
        let native_rx_counter = native_rx_counter.unwrap_or_default();
        let next_rx_header = if hardware_empty_pdu {
            // Hardware-authenticated empty PDUs drive NESN/SN and the pending
            // TX allocation, but are filtered before RX DMA and leave the
            // firmware ring cursor untouched.
            Some(header)
        } else {
            self.radio_write_guest_word(buffer, native_rx_counter as u32)?;
            self.radio_write_guest_word(buffer.wrapping_add(4), (native_rx_counter >> 32) as u32)?;
            self.radio_write_guest_bytes(buffer.wrapping_add(0x0c), &metadata)?;
            self.radio_write_guest_bytes(buffer.wrapping_add(0x1c), &dma_frame)?;
            // The baseband cursor names the next hardware-owned RX header.
            // Move it past the header just filled so the descriptor becomes
            // part of the completed prefix consumed by the recycle walk.
            let next_rx = self.radio_read_guest_word(header.wrapping_add(4))? & 0x000f_ffff;
            if next_rx == 0 {
                return Ok(false);
            }
            self.radio_write_guest_word(
                state.wrapping_add(8),
                (current_rx & !0x000f_ffff) | next_rx,
            )?;
            c6_ble_pointer(next_rx).and_then(|cursor| cursor.checked_sub(4))
        };
        if rx_buffer_identifier == 2 && !hardware_encryption_transition {
            // CURRENT_TX names the live list cursor while the event runs. A
            // successful peripheral RX/TX pair marks the current allocation
            // complete and leaves the cursor on it until task-context recycle
            // consumes the done bit. The recycle path reads that cursor from
            // link-state word zero rather than MMIO, so update both hardware
            // views atomically. CURRENT_RX advances to the next ring header at
            // the same completion edge.
            let completed_tx = self.native_ble_connection_tx_header(state)?;
            if let Some(completed_tx) = completed_tx {
                let tx_flags = self.radio_read_guest_word(completed_tx)?;
                self.radio_write_guest_word(completed_tx, tx_flags | 1)?;
                let link_state = self.radio_read_guest_word(state)?;
                self.radio_write_guest_word(
                    state,
                    (link_state & !0x000f_ffff)
                        | (completed_tx.wrapping_add(4) & 0x000f_ffff),
                )?;
            }
            handle.set_loaded_buffer_headers(schedule_address, completed_tx, next_rx_header);
        }
        if !hardware_empty_pdu {
            // Bit 27 records that RX DMA completed during this schedule.
            let schedule_flags = self.radio_read_guest_word(state.wrapping_add(0x14))?;
            self.radio_write_guest_word(state.wrapping_add(0x14), schedule_flags | (1 << 27))?;
        }

        let firmware_connection_response = if hardware_encryption_transition {
            None
        } else if rx_buffer_identifier == 2 {
            let tx_header = self.native_ble_connection_tx_header(state)?;
            match tx_header {
                Some(tx_header) => self.read_native_ble_pdu(state, tx_header, false)?,
                None => None,
            }
        } else {
            None
        };
        if rx_buffer_identifier == 2 {
            self.mark_native_ble_connection_rx_success(
                schedule_address,
                hardware_encryption_transition,
            )?;
        }
        let (connection_response, connection_tx_phy, continues_connection_event) =
            if rx_buffer_identifier == 2 {
            let sequence = self.radio_c6_ble_link_sequences.entry(state).or_default();
            let response_result = sequence.peripheral_response(frame, firmware_connection_response);
            let (response, response_error) = match response_result {
                Ok(response) => (response, None),
                Err(detail) => (None, Some(detail)),
            };
            let tx_phy = sequence.tx_phy();
            let valid_response = response.as_ref().is_some_and(|pdu| pdu.len() >= 2)
                || sequence.allows_silent_event_end();
            if hardware_empty_pdu {
                sequence.complete_hardware_filtered_rx();
            }
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::SchedulerState,
                    response_error.is_none() && valid_response,
                    self.now,
                    response_error.unwrap_or_else(|| {
                        format!(
                            "native connection schedule {schedule_address:#010x} produced an invalid peripheral response"
                        )
                    }),
                )?;
            (response, tx_phy, sequence.expects_central_response())
        } else {
            (None, "ble-1m", false)
        };

        self.radio_c6_ble_receptions.remove(activity_index);
        let successor = c6_ble_pointer(self.radio_read_guest_word(schedule_address)?);
        if let Some(bytes) = connection_response {
            let start = self
                .now
                .checked_add(SimDuration::from_ticks(C6_BLE_INTERFRAME_SPACE_TICKS))
                .map_err(|_| MachineError::TimeOverflow)?;
            let end = start
                .checked_add(frame_duration(bytes.len()))
                .map_err(|_| MachineError::TimeOverflow)?;
            let (response, completion_end) = if continues_connection_event {
                let response_start = end
                    .checked_add(SimDuration::from_ticks(C6_BLE_INTERFRAME_SPACE_TICKS))
                    .map_err(|_| MachineError::TimeOverflow)?;
                let response_end = response_start
                    .checked_add(SimDuration::from_ticks(
                        C6_BLE_INTERFRAME_SPACE_TICKS.saturating_mul(2),
                    ))
                    .map_err(|_| MachineError::TimeOverflow)?;
                let maximum_extension = activity
                    .end
                    .checked_add(SimDuration::from_ticks(
                        C6_BLE_INTERFRAME_SPACE_TICKS.saturating_mul(4),
                    ))
                    .map_err(|_| MachineError::TimeOverflow)?;
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::BluetoothLe,
                        RadioLegalityRule::SchedulerState,
                        response_end <= maximum_extension,
                        self.now,
                        format!(
                            "BLE encryption continuation ends at {response_end}, beyond the firmware-observed extension limit {maximum_extension}"
                        ),
                    )?;
                (
                    Some(PendingNativeBleReception {
                        start: response_start,
                        end: response_end,
                        schedule_address,
                        state,
                        spectrum: activity.spectrum,
                        phy: connection_tx_phy,
                        rx_buffer_identifier,
                    }),
                    response_end,
                )
            } else {
                (None, end)
            };
            let pending = PendingNativeBleTransmission {
                start,
                spectrum: activity.spectrum,
                phy: connection_tx_phy,
                bytes,
                response,
            };
            let insertion = self
                .radio_c6_pending_ble_transmissions
                .iter()
                .position(|queued| queued.start > start)
                .unwrap_or(self.radio_c6_pending_ble_transmissions.len());
            self.radio_c6_pending_ble_transmissions
                .insert(insertion, pending);
            handle.schedule_received_event_end(completion_end, schedule_address, successor);
        } else {
            handle.schedule_received_event_end(self.now, schedule_address, successor);
        }
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
        let rx_match = wifi_mac.rx_match_mask(frame.get(4..10).unwrap_or_default());
        if rx_match == 0 {
            return Ok(false);
        }
        let metadata = c6_wifi_rx_metadata(frame, rx_match, self.now);
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
                .expect("ESP32-C6 has a radio legality validator")
                .validate_coexistence_ownership(
                    match frame.protocol {
                        RadioProtocol::Wifi => RadioSubsystem::Wifi,
                        RadioProtocol::BluetoothLe => RadioSubsystem::BluetoothLe,
                        RadioProtocol::Ieee802154 => RadioSubsystem::Ieee802154,
                    },
                    frame.protocol,
                    granted_protocol,
                    self.now,
                )?;
            let transmission = self
                .radio_medium
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
            self.record_coexistence_transmission(grant, transmission);
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

    fn submit_native_wifi_receive_response(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
        frame: &[u8],
    ) -> Result<bool, MachineError> {
        if !self
            .esp32c6_peripherals
            .as_ref()
            .is_some_and(|handles| handles.modem.wifi_ready())
        {
            return Ok(false);
        }
        let ba_state = wifi_mac.validate_block_ack_sessions();
        self.radio_legality
            .as_mut()
            .expect("ESP32-C6 machine has a radio legality validator")
            .require(
                RadioSubsystem::Wifi,
                RadioLegalityRule::SchedulerState,
                ba_state.is_ok(),
                self.now,
                ba_state.err().unwrap_or_default(),
            )?;
        if let Some(mpdu) = crate::native_wifi::native_wifi_qos_mpdu(frame)
            && wifi_mac.rx_match_mask(&mpdu.receiver) != 0
        {
            wifi_mac.record_block_ack_mpdu(&mpdu.transmitter, mpdu.tid, mpdu.sequence);
        }
        let response = if let Some(request) =
            crate::native_wifi::native_wifi_block_ack_request(frame)
            && wifi_mac.rx_match_mask(&request.receiver) != 0
        {
            wifi_mac
                .block_ack_bitmap(&request.transmitter, request.tid, request.starting_sequence)
                .map(|bitmap| {
                    crate::native_wifi::native_wifi_block_ack_response(request, bitmap)
                })
        } else if frame
            .get(4..10)
            .is_some_and(|receiver| wifi_mac.rx_match_mask(receiver) != 0)
        {
            crate::native_wifi::native_wifi_immediate_response(frame)
        } else {
            None
        };
        let Some(bytes) = response else {
            return Ok(false);
        };
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
                protocol: RadioProtocol::Wifi,
                start: self.now,
                duration,
                priority: 15,
                preemptible: false,
            })?;
        let CoexistenceDecision::Granted {
            id: grant,
            protocol: granted_protocol,
            preempted,
            ..
        } = decision
        else {
            return Ok(false);
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
        self.record_coexistence_transmission(grant, transmission);
        Ok(true)
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
    // ESP32-C6 HP SRAM occupies 0x4080_0000..0x4088_0000. Native BLE words
    // reuse the next compressed-address bit for state, so accepting all twenty
    // low bits can turn a flagged non-pointer into an out-of-range DMA address.
    (low != 0 && low < 0x0008_0000).then_some(0x4080_0000 | low)
}

fn c6_wifi_rx_metadata(frame: &[u8], rx_match: u8, at: remu_core::SimTime) -> [u8; 92] {
    let mut metadata = [0_u8; 92];
    metadata[0] = (-40_i8) as u8;
    metadata[3] = (rx_match & 0x0f) << 4;
    metadata[11] = u8::from(frame.get(4).is_some_and(|address| address & 1 != 0)) << 7;
    metadata[12..16].copy_from_slice(&(at.ticks() as u32).to_le_bytes());
    metadata[20] = (-95_i8) as u8;
    metadata[21] = 1;
    let signal_length = frame.len().saturating_add(4).min(0x3fff) as u32;
    let dump_length = frame.len().min(0x3fff) as u32;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BleAuxPointer {
    channel: u8,
    offset_us: u16,
    phy: &'static str,
}

fn ble_aux_pointer(frame: &[u8]) -> Option<BleAuxPointer> {
    if frame.len() < 4 || frame[0] & 0x0f != 7 {
        return None;
    }
    let extended_header_length = usize::from(frame[2] & 0x3f);
    let extended_header_end = 3_usize.checked_add(extended_header_length)?;
    if extended_header_length == 0 || extended_header_end > frame.len() {
        return None;
    }
    let flags = frame[3];
    if flags & (1 << 4) == 0 {
        return None;
    }
    let mut cursor = 4_usize;
    for (bit, length) in [(0, 6_usize), (1, 6), (2, 1), (3, 2)] {
        if flags & (1 << bit) != 0 {
            cursor = cursor.checked_add(length)?;
        }
    }
    if cursor.checked_add(3)? > extended_header_end {
        return None;
    }
    let pointer = &frame[cursor..cursor + 3];
    let channel = pointer[0] & 0x3f;
    if channel > 36 {
        return None;
    }
    let offset_units_us = if pointer[0] & (1 << 7) != 0 {
        300_u16
    } else {
        30_u16
    };
    let offset = u16::from(pointer[1]) | (u16::from(pointer[2] & 0x1f) << 8);
    let phy = match pointer[2] >> 5 {
        0 => "ble-1m",
        1 => "ble-2m",
        2 => "ble-coded",
        _ => return None,
    };
    Some(BleAuxPointer {
        channel,
        offset_us: offset.saturating_mul(offset_units_us),
        phy,
    })
}

fn ble_data_spectrum(channel: u8) -> Spectrum {
    let center_khz = if channel <= 10 {
        2_404_000 + u32::from(channel) * 2_000
    } else {
        2_406_000 + u32::from(channel) * 2_000
    };
    Spectrum::new(center_khz, 2_000)
}

fn ble_data_channel_from_frequency_index(frequency_index: u8) -> Option<u8> {
    if !frequency_index.is_multiple_of(2) {
        return None;
    }
    match frequency_index {
        2..=22 => Some((frequency_index - 2) / 2),
        26..=76 => Some((frequency_index - 4) / 2),
        _ => None,
    }
}

fn frame_duration(length: usize) -> SimDuration {
    SimDuration::from_ticks(
        u64::try_from(length.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(32),
    )
}

#[cfg(test)]
mod ble_aux_tests {
    use super::*;

    #[test]
    fn decodes_channel_offset_and_phy() {
        let pointer =
            ble_aux_pointer(&[0x47, 0x07, 0x06, 0x18, 0x01, 0x30, 0x0d, 0x0f, 0x20])
                .expect("valid ADV_EXT_IND AuxPtr");
        assert_eq!(pointer.channel, 13);
        assert_eq!(pointer.offset_us, 450);
        assert_eq!(pointer.phy, "ble-2m");
        assert_eq!(ble_data_spectrum(pointer.channel).center_khz, 2_432_000);
    }

    #[test]
    fn maps_native_frequency_indices_around_advertising_channel_gaps() {
        assert_eq!(ble_data_channel_from_frequency_index(2), Some(0));
        assert_eq!(ble_data_channel_from_frequency_index(22), Some(10));
        assert_eq!(ble_data_channel_from_frequency_index(26), Some(11));
        assert_eq!(ble_data_channel_from_frequency_index(76), Some(36));
        assert_eq!(ble_data_channel_from_frequency_index(0), None);
        assert_eq!(ble_data_channel_from_frequency_index(24), None);
        assert_eq!(ble_data_channel_from_frequency_index(78), None);
        assert_eq!(ble_data_channel_from_frequency_index(13), None);
    }
}
