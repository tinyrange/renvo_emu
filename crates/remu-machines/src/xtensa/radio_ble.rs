impl XtensaMachine {
    fn submit_native_ble_frames(&mut self) -> Result<u64, XtensaMachineError> {
        while let Some(kick) = self.ble_exchange_memory.take_schedule_kick() {
            let slot = (kick.control & 0x0f) as u16;
            let slot_address =
                self.require_native_ble_mapping(slot.saturating_mul(16), "scheduler event slot")?;
            let cs_reference = self.require_native_ble_u16(
                slot_address.wrapping_add(8),
                "control-structure reference",
            )?;
            let coarse_low =
                self.require_native_ble_u16(slot_address.wrapping_add(2), "slot coarse clock low")?;
            let coarse_high = self
                .require_native_ble_u16(slot_address.wrapping_add(4), "slot coarse clock high")?;
            let fine =
                self.require_native_ble_u16(slot_address.wrapping_add(6), "slot fine clock")?;
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                u64::from(fine) < S3_BLE_FINE_POSITIONS_PER_HALF_SLOT,
                self.now,
                format!(
                    "scheduler fine clock {fine} is outside 0..{}",
                    S3_BLE_FINE_POSITIONS_PER_HALF_SLOT - 1
                ),
            )?;
            let coarse =
                (u64::from(coarse_low) | (u64::from(coarse_high) << 16)) & S3_BLE_COARSE_MASK;
            let tx_start = self.native_ble_slot_time(coarse, u64::from(fine))?;
            let cs_address = self.require_native_ble_mapping(
                cs_reference.saturating_mul(2),
                "scheduler control structure",
            )?;
            let access_address =
                self.require_native_ble_u32(cs_address.wrapping_add(12), "BLE access address")?;
            let event_word =
                self.require_native_ble_u16(cs_address.wrapping_add(2), "BLE event control")?;
            let event_index = s3_ble_event_index(event_word);
            let channel_word =
                self.require_native_ble_u16(cs_address.wrapping_add(22), "BLE channel")?;
            let channel = (channel_word & 0x3f) as u8;
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                channel <= 39,
                self.now,
                format!("scheduler selected invalid BLE channel {channel}"),
            )?;
            let tx_descriptor_offset = self
                .require_native_ble_u16(cs_address.wrapping_add(28), "TX descriptor reference")?;
            if tx_descriptor_offset == 0 {
                // A receive-only scan control structure has no TX descriptor.
                // Its window is expressed in 0.625-ms BLE slots at CS+0x20.
                // Record the actual RF aperture and complete the scheduler
                // event at the programmed end rather than interpreting the
                // receive control block as a packet descriptor.
                let window_units =
                    self.require_native_ble_u16(cs_address.wrapping_add(32), "RX window duration")?;
                self.radio_legality.require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::SchedulerState,
                    window_units != 0,
                    self.now,
                    "receive-only scheduler event has a zero-duration window",
                )?;
                let window_ticks = u64::from(window_units)
                    .saturating_mul(S3_BLE_HALF_SLOT_TICKS)
                    .saturating_mul(2);
                let end = tx_start
                    .checked_add(SimDuration::from_ticks(window_ticks))
                    .map_err(|_| XtensaMachineError::TimeOverflow)?;
                self.radio_medium.tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::BluetoothLe,
                    spectrum: s3_ble_spectrum(channel),
                    sensitivity_dbm: -100,
                })?;
                let insertion = self
                    .pending_native_ble_receptions
                    .iter()
                    .position(|pending| pending.start > tx_start.ticks())
                    .unwrap_or(self.pending_native_ble_receptions.len());
                self.pending_native_ble_receptions.insert(
                    insertion,
                    PendingNativeBleReception {
                        start: tx_start.ticks(),
                        end: end.ticks(),
                        slot_address,
                        event_index,
                        channel,
                        phy: "ble-1m",
                        complete_on_receive: access_address != BLE_ADVERTISING_ACCESS_ADDRESS,
                        response: None,
                    },
                );
                self.ble_exchange_memory
                    .schedule_radio_completion(end, S3_RWBLE_END_INTERRUPT);
                self.schedule_native_ble_slot_state(end, slot_address, 4);
                continue;
            }
            let tx_descriptor =
                self.require_native_ble_mapping(tx_descriptor_offset, "TX descriptor")?;
            let (mut pdu, extended) =
                self.read_native_ble_tx_pdu(cs_address, tx_descriptor, access_address)?;
            if access_address != BLE_ADVERTISING_ACCESS_ADDRESS {
                // A peripheral connection event is receive-first even though
                // the control structure already points at the PDU that will
                // be transmitted in response. The genuine controller opens
                // the RX aperture at the scheduler timestamp, then emits that
                // descriptor after the 150-us inter-frame spacing.
                let end = tx_start
                    .checked_add(SimDuration::from_ticks(36 * S3_BLE_1M_BYTE_TICKS))
                    .map_err(|_| XtensaMachineError::TimeOverflow)?;
                self.radio_medium.tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::BluetoothLe,
                    spectrum: s3_ble_spectrum(channel),
                    sensitivity_dbm: -100,
                })?;
                let phy_result = self
                    .native_ble_link_sequences
                    .entry(cs_address)
                    .or_default()
                    .begin_event();
                let (phys, phy_error) = match phy_result {
                    Ok(phys) => (Some(phys), None),
                    Err(detail) => (None, Some(detail)),
                };
                self.radio_legality.require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::SchedulerState,
                    phy_error.is_none(),
                    self.now,
                    phy_error.unwrap_or_default(),
                )?;
                let (tx_phy, rx_phy) =
                    phys.expect("legality check established BLE connection PHYs");
                let response = PendingNativeBleTransmission {
                    start: 0,
                    slot_address,
                    event_index,
                    channel,
                    phy: tx_phy,
                    complete_event: true,
                    response_window: false,
                    tx_interrupt: true,
                    deferred_descriptor: Some((cs_address, tx_descriptor, access_address)),
                    pdu,
                };
                let insertion = self
                    .pending_native_ble_receptions
                    .iter()
                    .position(|pending| pending.start > tx_start.ticks())
                    .unwrap_or(self.pending_native_ble_receptions.len());
                self.pending_native_ble_receptions.insert(
                    insertion,
                    PendingNativeBleReception {
                        start: tx_start.ticks(),
                        end: end.ticks(),
                        slot_address,
                        event_index,
                        channel,
                        phy: rx_phy,
                        complete_on_receive: false,
                        response: Some(response),
                    },
                );
                self.ble_exchange_memory
                    .schedule_radio_completion(end, S3_RWBLE_END_INTERRUPT);
                self.schedule_native_ble_slot_state(end, slot_address, 4);
                continue;
            }
            let mut complete_event = true;
            let mut auxiliary = None;
            if extended {
                let auxiliary_descriptor_offset = self.require_native_ble_u16(
                    cs_address.wrapping_add(52),
                    "auxiliary TX descriptor reference",
                )?;
                if auxiliary_descriptor_offset != 0 {
                    let auxiliary_descriptor = self.require_native_ble_mapping(
                        auxiliary_descriptor_offset,
                        "auxiliary TX descriptor",
                    )?;
                    let (auxiliary_pdu, auxiliary_extended) = self.read_native_ble_tx_pdu(
                        cs_address,
                        auxiliary_descriptor,
                        access_address,
                    )?;
                    self.radio_legality.require(
                        RadioSubsystem::BluetoothLe,
                        RadioLegalityRule::SchedulerState,
                        auxiliary_extended,
                        self.now,
                        "native auxiliary descriptor is not an extended advertising PDU",
                    )?;
                    let channel_control = self.require_native_ble_u16(
                        auxiliary_descriptor.wrapping_add(10),
                        "auxiliary channel control",
                    )?;
                    let offset_phy = self.require_native_ble_u16(
                        auxiliary_descriptor.wrapping_add(12),
                        "auxiliary offset and PHY control",
                    )?;
                    let auxiliary_channel = (channel_control & 0x3f) as u8;
                    let auxiliary_offset = offset_phy & 0x1fff;
                    let auxiliary_phy = match offset_phy >> 13 {
                        0 => Some("ble-1m"),
                        1 => Some("ble-2m"),
                        2 => Some("ble-coded"),
                        _ => None,
                    };
                    self.radio_legality.require(
                        RadioSubsystem::BluetoothLe,
                        RadioLegalityRule::SchedulerState,
                        auxiliary_channel <= 36
                            && auxiliary_offset != 0
                            && auxiliary_phy.is_some(),
                        self.now,
                        format!(
                            "native auxiliary control is invalid: channel={auxiliary_channel} offset={auxiliary_offset} phy={}",
                            offset_phy >> 13
                        ),
                    )?;
                    let replaced =
                        replace_s3_ble_aux_pointer(&mut pdu, channel_control as u8, offset_phy);
                    self.radio_legality.require(
                        RadioSubsystem::BluetoothLe,
                        RadioLegalityRule::SchedulerState,
                        replaced,
                        self.now,
                        "primary extended advertising PDU has no valid AuxPtr field",
                    )?;
                    let offset_unit_us = if channel_control & (1 << 7) != 0 {
                        300_u64
                    } else {
                        30_u64
                    };
                    let primary_end = tx_start
                        .checked_add(frame_duration(pdu.len()))
                        .map_err(|_| XtensaMachineError::TimeOverflow)?;
                    let auxiliary_start = primary_end
                        .checked_add(SimDuration::from_ticks(
                            u64::from(auxiliary_offset)
                                .saturating_mul(offset_unit_us)
                                .saturating_mul(16),
                        ))
                        .map_err(|_| XtensaMachineError::TimeOverflow)?;
                    auxiliary = Some(PendingNativeBleTransmission {
                        start: auxiliary_start.ticks(),
                        slot_address,
                        event_index,
                        channel: auxiliary_channel,
                        phy: auxiliary_phy.expect("legality check established auxiliary PHY"),
                        complete_event: true,
                        response_window: false,
                        tx_interrupt: false,
                        deferred_descriptor: None,
                        pdu: auxiliary_pdu,
                    });
                    complete_event = false;
                }
            }

            let insertion = self
                .pending_native_ble_transmissions
                .iter()
                .position(|pending| pending.start > tx_start.ticks())
                .unwrap_or(self.pending_native_ble_transmissions.len());
            self.pending_native_ble_transmissions.insert(
                insertion,
                PendingNativeBleTransmission {
                    start: tx_start.ticks(),
                    slot_address,
                    event_index,
                    channel,
                    phy: "ble-1m",
                    complete_event,
                    response_window: access_address == BLE_ADVERTISING_ACCESS_ADDRESS
                        && matches!(pdu.first().map(|header| header & 0x0f), Some(0) | Some(6)),
                    tx_interrupt: false,
                    deferred_descriptor: None,
                    pdu,
                },
            );
            if let Some(auxiliary) = auxiliary {
                let insertion = self
                    .pending_native_ble_transmissions
                    .iter()
                    .position(|pending| pending.start > auxiliary.start)
                    .unwrap_or(self.pending_native_ble_transmissions.len());
                self.pending_native_ble_transmissions
                    .insert(insertion, auxiliary);
            }
        }
        Ok(0)
    }

    fn read_native_ble_tx_pdu(
        &mut self,
        cs_address: u32,
        tx_descriptor: u32,
        access_address: u32,
    ) -> Result<(Vec<u8>, bool), XtensaMachineError> {
        let header = self.require_native_ble_u16(tx_descriptor.wrapping_add(2), "TX PDU header")?;
        let payload_offset =
            self.require_native_ble_u16(tx_descriptor.wrapping_add(4), "TX payload reference")?;
        let payload_address = self.require_native_ble_mapping(payload_offset, "TX payload")?;
        let header_byte = header as u8;
        let declared_length = usize::from((header >> 8) as u8);
        // The low data-channel header bits contain LLID, NESN, and SN.  A
        // perfectly ordinary connection PDU can therefore also have a low
        // nibble of seven; ADV_EXT_IND is only meaningful on the advertising
        // access address.
        let extended = access_address == BLE_ADVERTISING_ACCESS_ADDRESS && header_byte & 0x0f == 7;
        let advertising_pdu_with_local_address = access_address == BLE_ADVERTISING_ACCESS_ADDRESS
            && matches!(header_byte & 0x0f, 0 | 1 | 2 | 4 | 6);
        let payload_length = if advertising_pdu_with_local_address {
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                declared_length >= 6,
                self.now,
                format!(
                    "advertising PDU length {declared_length} cannot contain the six-byte local address"
                ),
            )?;
            declared_length - 6
        } else {
            declared_length
        };
        let mut pdu = Vec::with_capacity(2 + declared_length);
        pdu.extend_from_slice(&[header_byte, declared_length as u8]);
        if extended {
            let extended_header_length = usize::from(
                self.require_native_ble_bytes(
                    tx_descriptor.wrapping_add(6),
                    1,
                    "extended-header length",
                )?[0]
                    & 0x3f,
            );
            let flags = self.require_native_ble_bytes(
                tx_descriptor.wrapping_add(7),
                1,
                "extended-header flags",
            )?[0];
            let inserted_address_length: usize = if flags & 1 != 0 { 6 } else { 0 };
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                extended_header_length >= inserted_address_length.saturating_add(1)
                    && declared_length >= extended_header_length.saturating_add(1),
                self.now,
                "native extended advertising header has inconsistent lengths",
            )?;
            let sidecar_length = extended_header_length
                .saturating_add(1)
                .saturating_sub(inserted_address_length);
            let sidecar = self.require_native_ble_bytes(
                tx_descriptor.wrapping_add(6),
                sidecar_length,
                "extended advertising header sidecar",
            )?;
            pdu.extend_from_slice(&sidecar[..2]);
            if inserted_address_length != 0 {
                pdu.extend_from_slice(&self.require_native_ble_bytes(
                    cs_address.wrapping_add(6),
                    6,
                    "extended advertiser address",
                )?);
            }
            pdu.extend_from_slice(&sidecar[2..]);
            let advertising_data_length =
                declared_length.saturating_sub(extended_header_length.saturating_add(1));
            pdu.extend_from_slice(&self.require_native_ble_bytes(
                payload_address,
                advertising_data_length,
                "extended advertising data",
            )?);
        } else {
            if advertising_pdu_with_local_address {
                pdu.extend_from_slice(&self.require_native_bluetooth_address()?);
            }
            pdu.extend_from_slice(&self.require_native_ble_bytes(
                payload_address,
                payload_length,
                "TX PDU payload",
            )?);
        }
        Ok((pdu, extended))
    }

    fn service_pending_native_ble_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut submitted = 0_u64;
        while self
            .pending_native_ble_transmissions
            .front()
            .is_some_and(|pending| pending.start <= self.now.ticks())
        {
            let Some(mut pending) = self.pending_native_ble_transmissions.pop_front() else {
                break;
            };
            if let Some((cs_address, descriptor, access_address)) = pending.deferred_descriptor {
                let (pdu, extended) =
                    self.read_native_ble_tx_pdu(cs_address, descriptor, access_address)?;
                self.radio_legality.require(
                    RadioSubsystem::BluetoothLe,
                    RadioLegalityRule::SchedulerState,
                    !extended,
                    self.now,
                    "connection response references an extended advertising descriptor",
                )?;
                pending.pdu = pdu;
            }
            let duration = frame_duration(pending.pdu.len());
            let continue_connection_event = pending.tx_interrupt
                && pending.pdu.first().is_some_and(|header| header & 0x10 != 0);
            let decision = self.radio_coexistence.request(CoexistenceRequest {
                protocol: RadioProtocol::BluetoothLe,
                start: self.now,
                duration,
                priority: 9,
                preemptible: true,
            })?;
            let due = self
                .now
                .checked_add(duration)
                .map_err(|_| XtensaMachineError::TimeOverflow)?;
            if let CoexistenceDecision::Granted {
                id: grant,
                protocol: granted_protocol,
                preempted,
                ..
            } = decision
            {
                self.apply_coexistence_preemption(preempted)?;
                self.radio_legality.validate_coexistence_ownership(
                    RadioSubsystem::BluetoothLe,
                    RadioProtocol::BluetoothLe,
                    granted_protocol,
                    self.now,
                )?;
                let transmission = self.radio_medium.transmit(TxRequest {
                    source: EMULATED_NODE,
                    start: self.now,
                    end: due,
                    power_dbm: 0,
                    frame: RadioFrame {
                        protocol: RadioProtocol::BluetoothLe,
                        spectrum: s3_ble_spectrum(pending.channel),
                        phy: pending.phy.to_owned(),
                        bytes: pending.pdu,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
                self.record_coexistence_transmission(grant, transmission);
                // A legacy advertising slot does not request a standalone TX
                // callback. Raising RWBLE's global TX cause here would make
                // the scheduler deliver status 3 to lld_adv, which is invalid
                // for this slot type. Hardware reports the completed event via
                // END after the inter-frame response window instead.
                self.schedule_native_ble_slot_state(due, pending.slot_address, 2);
                if pending.tx_interrupt {
                    self.ble_exchange_memory
                        .schedule_radio_completion(due, S3_RWBLE_TX_INTERRUPT);
                }
                if pending.complete_event {
                    let response_start = due
                        .checked_add(SimDuration::from_ticks(S3_BLE_INTERFRAME_SPACE_TICKS))
                        .map_err(|_| XtensaMachineError::TimeOverflow)?;
                    let end_due = if continue_connection_event {
                        let response_end = response_start
                            .checked_add(SimDuration::from_ticks(36 * S3_BLE_1M_BYTE_TICKS))
                            .map_err(|_| XtensaMachineError::TimeOverflow)?;
                        self.radio_medium.tune_receiver(Receiver {
                            node: EMULATED_NODE,
                            protocol: RadioProtocol::BluetoothLe,
                            spectrum: s3_ble_spectrum(pending.channel),
                            sensitivity_dbm: -100,
                        })?;
                        let insertion = self
                            .pending_native_ble_receptions
                            .iter()
                            .position(|reception| reception.start > response_start.ticks())
                            .unwrap_or(self.pending_native_ble_receptions.len());
                        self.pending_native_ble_receptions.insert(
                            insertion,
                            PendingNativeBleReception {
                                start: response_start.ticks(),
                                end: response_end.ticks(),
                                slot_address: pending.slot_address,
                                event_index: pending.event_index,
                                channel: pending.channel,
                                phy: pending.phy,
                                // Connection RX is reported before the frame
                                // event closes. Preserve the separately
                                // scheduled END so the ROM can consume RX and
                                // update link state first.
                                complete_on_receive: false,
                                response: None,
                            },
                        );
                        response_end
                    } else if pending.response_window {
                        let response_end = response_start
                            .checked_add(SimDuration::from_ticks(36 * S3_BLE_1M_BYTE_TICKS))
                            .map_err(|_| XtensaMachineError::TimeOverflow)?;
                        self.radio_medium.tune_receiver(Receiver {
                            node: EMULATED_NODE,
                            protocol: RadioProtocol::BluetoothLe,
                            spectrum: s3_ble_spectrum(pending.channel),
                            sensitivity_dbm: -100,
                        })?;
                        let insertion = self
                            .pending_native_ble_receptions
                            .iter()
                            .position(|reception| reception.start > response_start.ticks())
                            .unwrap_or(self.pending_native_ble_receptions.len());
                        self.pending_native_ble_receptions.insert(
                            insertion,
                            PendingNativeBleReception {
                                start: response_start.ticks(),
                                end: response_end.ticks(),
                                slot_address: pending.slot_address,
                                event_index: pending.event_index,
                                channel: pending.channel,
                                phy: pending.phy,
                                complete_on_receive: true,
                                response: None,
                            },
                        );
                        response_end
                    } else {
                        response_start
                    };
                    self.ble_exchange_memory
                        .schedule_radio_completion(end_due, S3_RWBLE_END_INTERRUPT);
                    self.schedule_native_ble_slot_state(end_due, pending.slot_address, 4);
                }
                submitted = submitted.saturating_add(1);
            } else {
                self.ble_exchange_memory
                    .schedule_radio_completion(due, S3_RWBLE_SKIP_INTERRUPT);
            }
        }
        Ok(submitted)
    }

    fn native_ble_slot_time(
        &mut self,
        coarse: u64,
        fine: u64,
    ) -> Result<remu_core::SimTime, XtensaMachineError> {
        let fine = fine.min(S3_BLE_FINE_POSITIONS_PER_HALF_SLOT - 1);
        let target_in_cycle = coarse * S3_BLE_HALF_SLOT_TICKS
            + (S3_BLE_FINE_POSITIONS_PER_HALF_SLOT - 1 - fine) * S3_BLE_FINE_POSITION_TICKS;
        let now_in_cycle = self.now.ticks() % S3_BLE_CLOCK_CYCLE_TICKS;
        let delta = target_in_cycle
            .wrapping_add(S3_BLE_CLOCK_CYCLE_TICKS)
            .wrapping_sub(now_in_cycle)
            % S3_BLE_CLOCK_CYCLE_TICKS;
        if delta > S3_BLE_CLOCK_CYCLE_TICKS / 2 {
            let late_by = S3_BLE_CLOCK_CYCLE_TICKS - delta;
            let maximum_observed_lateness = 8 * S3_BLE_HALF_SLOT_TICKS;
            self.radio_legality.require(
                RadioSubsystem::BluetoothLe,
                RadioLegalityRule::SchedulerState,
                late_by <= maximum_observed_lateness,
                self.now,
                format!(
                    "BLE scheduler kick is {late_by} ticks late; genuine firmware stays within {maximum_observed_lateness} ticks"
                ),
            )?;
            return Ok(self.now);
        }
        Ok(remu_core::SimTime::from_ticks(
            self.now.ticks().saturating_add(delta),
        ))
    }

    fn schedule_native_ble_slot_state(
        &mut self,
        due: remu_core::SimTime,
        slot_address: u32,
        state: u16,
    ) {
        let insertion = self
            .pending_native_ble_slot_completions
            .iter()
            .position(|(existing, _, _)| *existing > due.ticks())
            .unwrap_or(self.pending_native_ble_slot_completions.len());
        self.pending_native_ble_slot_completions
            .insert(insertion, (due.ticks(), slot_address, state));
    }

    fn complete_native_ble_slot_states(&mut self) -> Result<(), XtensaMachineError> {
        while self
            .pending_native_ble_slot_completions
            .front()
            .is_some_and(|(due, _, _)| self.now.ticks() >= *due)
        {
            let Some((_, slot_address, state)) =
                self.pending_native_ble_slot_completions.pop_front()
            else {
                break;
            };
            self.set_native_ble_slot_state(slot_address, state)?;
        }
        Ok(())
    }

    fn set_native_ble_slot_state(
        &mut self,
        slot_address: u32,
        state: u16,
    ) -> Result<(), XtensaMachineError> {
        let control = self.require_native_ble_u16(slot_address, "completed scheduler slot")?;
        // RWBLE owns event-table state bits 3:5 after firmware starts a slot.
        // State 2 denotes a completed RX/TX frame; state 4 denotes successful
        // event completion for END. Firmware owns every other command field.
        let completed = (control & !0x0038) | (state << 3);
        self.bus.write(
            u64::from(slot_address),
            AccessWidth::HalfWord,
            u64::from(completed),
            self.now,
        )?;
        Ok(())
    }

    fn require_native_ble_mapping(
        &mut self,
        exchange_offset: u16,
        label: &str,
    ) -> Result<u32, XtensaMachineError> {
        let address = self.ble_exchange_memory.resolve_em_address(exchange_offset);
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::MemoryMapping,
            address.is_some(),
            self.now,
            format!(
                "{label} exchange-memory offset {exchange_offset:#06x} has no firmware-programmed mapping"
            ),
        )?;
        Ok(address.expect("legality check established an exchange-memory mapping"))
    }

    fn require_native_ble_u16(
        &mut self,
        address: u32,
        label: &str,
    ) -> Result<u16, XtensaMachineError> {
        let value = self.read_native_ble_u16(address);
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::MemoryMapping,
            value.is_some(),
            self.now,
            format!("{label} at {address:#010x} is not readable guest memory"),
        )?;
        Ok(value.expect("legality check established a readable halfword"))
    }

    fn require_native_ble_u32(
        &mut self,
        address: u32,
        label: &str,
    ) -> Result<u32, XtensaMachineError> {
        let value = self.read_native_ble_u32(address);
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::MemoryMapping,
            value.is_some(),
            self.now,
            format!("{label} at {address:#010x} is not readable guest memory"),
        )?;
        Ok(value.expect("legality check established a readable word"))
    }

    fn require_native_ble_bytes(
        &mut self,
        address: u32,
        length: usize,
        label: &str,
    ) -> Result<Vec<u8>, XtensaMachineError> {
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::SchedulerState,
            length <= u8::MAX as usize,
            self.now,
            format!("{label} length {length} exceeds the recovered eight-bit field"),
        )?;
        let bytes = self.read_native_ble_bytes(address, length);
        self.radio_legality.require(
            RadioSubsystem::BluetoothLe,
            RadioLegalityRule::MemoryMapping,
            bytes.is_some(),
            self.now,
            format!(
                "{label} range {address:#010x}..{:#010x} is not readable guest memory",
                address.wrapping_add(length as u32)
            ),
        )?;
        Ok(bytes.expect("legality check established a readable byte range"))
    }

    fn read_native_ble_u16(&mut self, address: u32) -> Option<u16> {
        self.bus
            .read(
                u64::from(address),
                AccessWidth::HalfWord,
                AccessKind::Read,
                self.now,
            )
            .ok()
            .map(|value| value as u16)
    }

    fn read_native_ble_u32(&mut self, address: u32) -> Option<u32> {
        self.bus
            .read(
                u64::from(address),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )
            .ok()
            .map(|value| value as u32)
    }

    fn read_native_ble_bytes(&mut self, address: u32, length: usize) -> Option<Vec<u8>> {
        (length <= 255)
            .then(|| {
                (0..length)
                    .map(|offset| {
                        self.bus
                            .read(
                                u64::from(address.wrapping_add(offset as u32)),
                                AccessWidth::Byte,
                                AccessKind::Read,
                                self.now,
                            )
                            .ok()
                            .map(|value| value as u8)
                    })
                    .collect()
            })
            .flatten()
    }

    fn require_native_bluetooth_address(&mut self) -> Result<[u8; 6], XtensaMachineError> {
        let low = self.require_native_ble_u32(0x6000_7044, "factory Bluetooth address low")?;
        let high = self.require_native_ble_u32(0x6000_7048, "factory Bluetooth address high")?;
        let mut address = [0_u8; 6];
        address[..4].copy_from_slice(&low.to_le_bytes());
        address[4..].copy_from_slice(&high.to_le_bytes()[..2]);
        // ESP32-S3 derives the Bluetooth universal address from the factory
        // base address by adding two to the least-significant octet.
        address[0] = address[0].wrapping_add(2);
        Ok(address)
    }

}
