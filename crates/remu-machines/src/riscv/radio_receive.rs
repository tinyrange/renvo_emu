impl RiscVMachine {
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
                (
                    DeliveryOutcome::Collision { .. } | DeliveryOutcome::SeededLoss,
                    Some((frame, _)),
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
                protocol: granted_protocol,
                ..
            } = decision
            else {
                continue;
            };
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

fn frame_duration(length: usize) -> SimDuration {
    SimDuration::from_ticks(
        u64::try_from(length.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(32),
    )
}
