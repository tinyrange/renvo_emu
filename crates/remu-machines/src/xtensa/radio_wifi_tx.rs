impl XtensaMachine {
    fn submit_native_wifi_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut submitted = 0_u64;
        while let Some(descriptor) = self.wifi_mac.take_tx_descriptor() {
            self.radio_legality.validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                descriptor.address,
                4,
                8,
                8,
                self.now,
            )?;
            let control = self.bus.read(
                u64::from(descriptor.address),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let buffer = self.bus.read(
                u64::from(descriptor.address.wrapping_add(4)),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )?;
            let capacity = (control as usize) & 0x0fff;
            let wire_length = ((control as usize) >> 12) & 0x0fff;
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                remu_radio::RadioLegalityRule::DmaLength,
                wire_length > 4,
                self.now,
                format!(
                    "TX DMA wire length {wire_length} does not contain a MAC frame and 4-byte FCS"
                ),
            )?;
            // The S3 MAC length includes a hardware-generated four-byte FCS,
            // while descriptor capacity covers only guest-owned frame bytes.
            // Genuine authentication traffic uses exactly capacity+4.
            let length = wire_length - 4;
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                remu_radio::RadioLegalityRule::DmaLength,
                length <= capacity,
                self.now,
                format!("TX DMA MAC-frame length {length} exceeds descriptor capacity {capacity}"),
            )?;
            self.radio_legality.validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                buffer as u32,
                2,
                length,
                4095,
                self.now,
            )?;
            let mut bytes = (0..length)
                .map(|offset| {
                    self.bus
                        .read(
                            u64::from((buffer as u32).wrapping_add(offset as u32)),
                            AccessWidth::Byte,
                            AccessKind::Read,
                            self.now,
                        )
                        .map(|value| value as u8)
                })
                .collect::<Result<Vec<_>, _>>()?;
            // Genuine firmware consumes its intermediate bit-29 security mark
            // before the final queue kick. Protected Frame plus the CCMP
            // ExtIV/key-ID header is the durable request seen by the MAC.
            let protection_requested = control & (1 << 29) != 0
                || bytes.get(1).is_some_and(|flags| flags & 0x40 != 0);
            if protection_requested {
                let key = self.wifi_mac.select_ccmp_tx_key(&bytes);
                self.radio_legality.require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CryptoKeySelection,
                    key.is_ok(),
                    self.now,
                    key.as_ref().err().cloned().unwrap_or_default(),
                )?;
                let protected = remu_radio::protect_native_ccmp_frame(
                    &key.expect("legality accepted the native CCMP key"),
                    &mut bytes,
                );
                self.radio_legality.require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CryptoKeySelection,
                    protected.is_ok(),
                    self.now,
                    protected.err().map(|error| error.to_string()).unwrap_or_default(),
                )?;
            }
            let duration = frame_duration(bytes.len());
            let end = self
                .now
                .checked_add(duration)
                .map_err(|_| XtensaMachineError::TimeOverflow)?;
            let pending = crate::native_wifi::PendingNativeWifiTransmission::new(
                descriptor.queue,
                &bytes,
                end,
                self.wifi_mac.tx_ack_timeout(descriptor.queue),
            )
            .ok_or(XtensaMachineError::TimeOverflow)?;
            let decision = self.radio_coexistence.request(CoexistenceRequest {
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
                self.radio_legality.require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CompletionWithoutOperation,
                    self.wifi_mac.complete_tx(
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
            self.radio_legality.validate_coexistence_ownership(
                RadioSubsystem::Wifi,
                RadioProtocol::Wifi,
                granted_protocol,
                self.now,
            )?;
            let transmission = self.radio_medium.transmit(TxRequest {
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
            if pending.expected_response.is_some() {
                self.radio_medium.tune_receiver(Receiver {
                    node: EMULATED_NODE,
                    protocol: RadioProtocol::Wifi,
                    spectrum: Spectrum::new(2_412_000, 20_000),
                    sensitivity_dbm: -100,
                })?;
            }
            self.pending_native_wifi.push(pending);
            self.record_coexistence_transmission(grant, transmission);
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }
}
