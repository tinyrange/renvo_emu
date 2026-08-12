impl RiscVMachine {
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
            let buffer = buffer as u32;
            let wire_word = self.bus.read(
                u64::from(buffer),
                AccessWidth::Word,
                AccessKind::Read,
                self.now,
            )? as u32;
            // The pinned C6 LMAC writes the 12-bit PSDU length in the low
            // field.  Its genuine HE/TWT path also sets bit 24 before handing
            // the same buffer to the native queue; disassembly shows LMAC
            // tests descriptor flags independently of the length passed to
            // hal_mac_tx_set_ppdu.  Preserve that observed flag boundary and
            // reject every other high-bit encoding instead of silently
            // interpreting flags as a giant DMA transfer.
            const WIRE_LENGTH_MASK: u32 = 0x0fff;
            const OBSERVED_WIRE_FLAGS: u32 = 1 << 24;
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    remu_radio::RadioLegalityRule::DmaLength,
                    wire_word & !(WIRE_LENGTH_MASK | OBSERVED_WIRE_FLAGS) == 0,
                    self.now,
                    format!("TX DMA wire word {wire_word:#010x} has unobserved flag bits"),
                )?;
            let wire_length = (wire_word & WIRE_LENGTH_MASK) as usize;
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
            let mut bytes = self.radio_read_guest_bytes(buffer.wrapping_add(8), length)?;
            // The vendor preparation path briefly marks an intermediate DMA
            // word with bit 29, then consumes it while expanding the final
            // queue descriptor by the eight-byte CCMP header. At the native
            // queue boundary the durable hardware request is Protected Frame
            // plus ExtIV/key ID. Accept the direct bit-29 form as well.
            let protection_requested = control & (1 << 29) != 0
                || bytes.get(1).is_some_and(|flags| flags & 0x40 != 0);
            if protection_requested {
                let key = wifi_mac.select_ccmp_tx_key(&bytes);
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
                    &mut bytes,
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
            let duration = frame_duration(bytes.len());
            let end = self
                .now
                .checked_add(duration)
                .map_err(|_| MachineError::TimeOverflow)?;
            let pending = crate::native_wifi::PendingNativeWifiTransmission::new(
                descriptor.queue,
                &bytes,
                end,
                wifi_mac.tx_ack_timeout(descriptor.queue),
            )
            .ok_or(MachineError::TimeOverflow)?;
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
                        phy: "wifi-ht20".to_owned(),
                        bytes,
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
}
