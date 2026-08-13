impl RiscVMachine {
    fn submit_native_wifi_frames(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
    ) -> Result<u64, MachineError> {
        let mut submitted = 0_u64;
        while let Some(descriptor) = wifi_mac.take_tx_descriptor() {
            let mut frames = self.read_native_wifi_tx_chain(descriptor.address)?;
            // Genuine firmware also sets descriptor bit 29 for an unprotected
            // QoS data frame. The durable hardware-crypto request at this
            // boundary is therefore the MAC Protected Frame bit together with
            // the CCMP ExtIV/key-ID header, not descriptor bit 29 by itself.
            for (_, bytes) in &mut frames {
                let protection_requested =
                    bytes.get(1).is_some_and(|flags| flags & 0x40 != 0);
                if !protection_requested {
                    continue;
                }
                let key = wifi_mac.select_ccmp_tx_key(bytes);
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::Wifi,
                        RadioLegalityRule::CryptoKeySelection,
                        key.is_ok(),
                        self.now,
                        key.as_ref().err().cloned().unwrap_or_default(),
                    )?;
                let protected = remu_radio::protect_native_ccmp_frame(
                    &key.expect("legality accepted the native CCMP key"),
                    bytes,
                );
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::Wifi,
                        RadioLegalityRule::CryptoKeySelection,
                        protected.is_ok(),
                        self.now,
                        protected.err().map(|error| error.to_string()).unwrap_or_default(),
                    )?;
            }
            let mut frames = frames
                .into_iter()
                .map(|(_, frame)| frame)
                .collect::<Vec<_>>();
            let aggregate = frames.len() > 1;
            let mut protected_payload = None;
            let bytes = if wifi_mac.tx_rts_enabled(descriptor.queue) {
                let rts = crate::native_wifi::native_wifi_rts_protection(&frames[0]);
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::Wifi,
                        remu_radio::RadioLegalityRule::SchedulerState,
                        rts.is_some(),
                        self.now,
                        format!(
                            "native TX queue {} requested RTS for a frame without a legal unicast response exchange",
                            descriptor.queue
                        ),
                    )?;
                let rts = rts.expect("legality accepted descriptor-driven RTS");
                protected_payload = Some(frames[0].clone());
                rts
            } else {
                Vec::new()
            };
            let rts_protected = protected_payload.is_some();
            let airtime_length = if rts_protected {
                bytes.len()
            } else {
                frames.iter().map(Vec::len).sum()
            };
            let duration = frame_duration(airtime_length);
            let end = self
                .now
                .checked_add(duration)
                .map_err(|_| MachineError::TimeOverflow)?;
            let mut pending = if rts_protected {
                crate::native_wifi::PendingNativeWifiTransmission::new(
                    descriptor.queue,
                    &bytes,
                    protected_payload,
                    end,
                    wifi_mac.tx_ack_timeout(descriptor.queue),
                )
                .ok_or(MachineError::TimeOverflow)?
            } else if aggregate {
                crate::native_wifi::PendingNativeWifiTransmission::new_aggregate(
                    descriptor.queue,
                    &frames,
                    end,
                    wifi_mac.tx_ack_timeout(descriptor.queue),
                )
                .map_err(|detail| remu_radio::RadioLegalityError {
                    chip: remu_radio::RadioChip::Esp32C6,
                    subsystem: RadioSubsystem::Wifi,
                    rule: RadioLegalityRule::SchedulerState,
                    at: self.now,
                    detail,
                })?
            } else {
                crate::native_wifi::PendingNativeWifiTransmission::new(
                    descriptor.queue,
                    &frames[0],
                    None,
                    end,
                    wifi_mac.tx_ack_timeout(descriptor.queue),
                )
                .ok_or(MachineError::TimeOverflow)?
            };
            if rts_protected && aggregate {
                let protected = pending.protect_aggregate(std::mem::take(&mut frames));
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::Wifi,
                        RadioLegalityRule::SchedulerState,
                        protected.is_ok(),
                        self.now,
                        protected.err().unwrap_or_default(),
                    )?;
            }
            let (bytes, mpdus, phy) = if rts_protected {
                (bytes, Vec::new(), "wifi-ht20")
            } else if aggregate {
                (Vec::new(), std::mem::take(&mut frames), "wifi-ht20-ampdu")
            } else {
                (frames.remove(0), Vec::new(), "wifi-ht20")
            };
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
                self.radio_legality
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio legality validator")
                    .require(
                        RadioSubsystem::Wifi,
                        RadioLegalityRule::CompletionWithoutOperation,
                        wifi_mac.complete_tx(
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
                        phy: phy.to_owned(),
                        bytes,
                        mpdus,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
            if pending.expected_response.is_some() {
                self.radio_medium
                    .as_mut()
                    .expect("ESP32-C6 machine has a radio medium")
                    .tune_receiver(Receiver {
                        node: EMULATED_NODE,
                        protocol: RadioProtocol::Wifi,
                        spectrum: Spectrum::new(2_412_000, 20_000),
                        sensitivity_dbm: -100,
                    })?;
            }
            self.radio_pending_native_wifi.push(pending);
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }

    /// Decodes the hardware-owned descriptor chain assembled by pinned LMAC.
    /// `ppAdd2AMPDUTail` writes the next MPDU head into the prior tail's word
    /// two. Every descriptor remains hardware-owned, intermediate MPDUs have
    /// EOF clear, and only the terminal MPDU has EOF set.
    fn read_native_wifi_tx_chain(
        &mut self,
        first: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>, MachineError> {
        const DMA_OWNER: u32 = 1 << 31;
        const DMA_EOF: u32 = 1 << 30;
        const WIRE_LENGTH_MASK: u32 = 0x0fff;
        let mut address = first;
        let mut visited = Vec::new();
        let mut frames = Vec::new();
        loop {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::SchedulerState,
                    visited.len() < 64 && !visited.contains(&address),
                    self.now,
                    format!(
                        "native TX descriptor chain is cyclic or exceeds 64 MPDUs at {address:#010x}"
                    ),
                )?;
            visited.push(address);
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .validate_dma(
                    RadioSubsystem::Wifi,
                    RadioDmaDirection::Transmit,
                    address,
                    4,
                    12,
                    12,
                    self.now,
                )?;
            let control = self.bus.read(
                u64::from(address),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )? as u32;
            let buffer = self.bus.read(
                u64::from(address.wrapping_add(4)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )? as u32;
            let next = self.bus.read(
                u64::from(address.wrapping_add(8)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )? as u32;
            let expected_eof = if next == 0 { DMA_EOF } else { 0 };
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::SchedulerState,
                    control & (DMA_OWNER | DMA_EOF) == DMA_OWNER | expected_eof,
                    self.now,
                    format!(
                        "native TX descriptor {address:#010x} has owner/EOF state {control:#010x} inconsistent with next pointer {next:#010x}"
                    ),
                )?;
            let wire_word = self.bus.read(
                u64::from(buffer),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )? as u32;
            let wire_length = (wire_word & WIRE_LENGTH_MASK) as usize;
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::DmaLength,
                    wire_length > 4,
                    self.now,
                    format!(
                        "TX DMA wire length {wire_length} does not contain a MAC frame and 4-byte FCS"
                    ),
                )?;
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
            let frame = self.radio_read_guest_bytes(buffer.wrapping_add(8), length)?;
            frames.push((control, frame));
            if next == 0 {
                return Ok(frames);
            }
            address = next;
        }
    }

    fn continue_native_wifi_after_cts(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
        mut pending: crate::native_wifi::PendingNativeWifiTransmission,
    ) -> Result<Option<crate::native_wifi::PendingNativeWifiTransmission>, MachineError> {
        let length = pending
            .protected_payload_len()
            .expect("CTS continuation has a deferred native Wi-Fi payload");
        let duration = frame_duration(length);
        let end = self
            .now
            .checked_add(duration)
            .map_err(|_| MachineError::TimeOverflow)?;
        let frames = pending
            .begin_protected_payload(end)
            .ok_or(MachineError::TimeOverflow)?;
        let (bytes, mpdus, phy) = if frames.len() == 1 {
            (frames.into_iter().next().unwrap(), Vec::new(), "wifi-ht20")
        } else {
            (Vec::new(), frames, "wifi-ht20-ampdu")
        };
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
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CompletionWithoutOperation,
                    wifi_mac.complete_tx(
                        pending.queue,
                        remu_devices::EspWifiTxOutcome::TransmitError,
                    ),
                    self.now,
                    format!(
                        "RTS-protected native TX queue {} rejected its RF-arbitration failure completion",
                        pending.queue
                    ),
                )?;
            return Ok(None);
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
                    phy: phy.to_owned(),
                    bytes,
                    mpdus,
                    origin: FrameOrigin::Emulated,
                },
            })?;
        if pending.expected_response.is_some() {
            self.radio_medium
                .as_mut()
                .expect("ESP32-C6 machine has a radio medium")
                .tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::Wifi,
                    spectrum: Spectrum::new(2_412_000, 20_000),
                    sensitivity_dbm: -100,
                })?;
        }
        self.record_coexistence_transmission(grant, transmission);
        Ok(Some(pending))
    }
}
