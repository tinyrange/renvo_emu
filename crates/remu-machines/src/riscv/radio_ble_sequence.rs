impl C6BleLinkSequence {
    pub(super) fn begin_event(&mut self) -> Result<&'static str, String> {
        self.active_event = self.event_counter;
        if let Some(update) = self.pending_phy_update {
            if update.instant == self.active_event {
                self.tx_phy = update.tx_phy;
                self.rx_phy = update.rx_phy;
                self.pending_phy_update = None;
            } else if self.active_event.wrapping_sub(update.instant) < 0x8000 {
                return Err(format!(
                    "BLE PHY update instant {} passed at connection event {}",
                    update.instant, self.active_event
                ));
            }
        }
        self.event_counter = self.event_counter.wrapping_add(1);
        Ok(self.rx_phy)
    }

    pub(super) fn tx_phy(&self) -> &'static str {
        self.tx_phy
    }

    pub(super) fn expects_central_response(&self) -> bool {
        self.encryption_phase == C6BleEncryptionPhase::StartReqSent
    }

    pub(super) fn hardware_filters_empty(&self, received: &[u8]) -> Result<bool, String> {
        if self.encryption_phase == C6BleEncryptionPhase::SessionKeyReady {
            return Ok(received.get(1).copied() == Some(0));
        }
        if self.rx_encryption_active() {
            return self
                .decode_received(received)
                .map(|plaintext| plaintext.get(1).copied() == Some(0));
        }
        Ok(false)
    }

    pub(super) fn native_rx_dma_frame(&self, received: &[u8]) -> Result<Vec<u8>, String> {
        if self.encryption_phase == C6BleEncryptionPhase::Encrypted {
            self.decode_received(received)
        } else {
            Ok(received.to_vec())
        }
    }

    pub(super) fn allows_silent_event_end(&self) -> bool {
        self.encryption_phase == C6BleEncryptionPhase::StartRspReceived
    }

    pub(super) fn complete_hardware_filtered_rx(&mut self) {
        // Authenticated empty PDUs update the baseband's ACK and packet-counter
        // state but never reach the firmware RX ring or its explicit CCM path.
        self.pending_native_rx_counter = None;
    }

    fn encryption_material(&self) -> Result<([u8; 16], [u8; 8]), String> {
        let Some(key) = self.session_key else {
            return Err(
                "BLE encryption is active without a firmware-derived session key".to_owned(),
            );
        };
        let Some(iv) = self.encryption_iv else {
            return Err("BLE encryption is active without a negotiated IV".to_owned());
        };
        Ok((key, iv))
    }

    fn rx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            C6BleEncryptionPhase::StartReqSent
                | C6BleEncryptionPhase::StartRspReceived
                | C6BleEncryptionPhase::Encrypted
        )
    }

    fn tx_encryption_active(&self) -> bool {
        matches!(
            self.encryption_phase,
            C6BleEncryptionPhase::StartRspReceived | C6BleEncryptionPhase::Encrypted
        )
    }

    pub(super) fn decode_received(&self, received: &[u8]) -> Result<Vec<u8>, String> {
        if !self.rx_encryption_active() {
            return Ok(received.to_vec());
        }
        let Some(header) = received.first().copied() else {
            return Err("encrypted BLE connection PDU has no header".to_owned());
        };
        let received_new = (header & (1 << 3) != 0) == self.expected_rx_sn;
        let counter = if received_new {
            self.rx_packet_counter
        } else {
            self.rx_packet_counter.checked_sub(1).ok_or_else(|| {
                "encrypted BLE retransmission precedes packet counter zero".to_owned()
            })?
        };
        let (key, iv) = self.encryption_material()?;
        ble_link_decrypt_pdu(
            &key,
            &iv,
            counter,
            BleLinkDirection::CentralToPeripheral,
            received,
        )
        .map_err(|error| format!("encrypted BLE RX counter {counter}: {error}"))
    }

    pub(super) fn native_rx_packet_counter(&self, received: &[u8]) -> Result<Option<u64>, String> {
        if !self.rx_encryption_active() {
            return Ok(None);
        }
        let Some(header) = received.first().copied() else {
            return Err("encrypted BLE connection PDU has no header".to_owned());
        };
        let received_new = (header & (1 << 3) != 0) == self.expected_rx_sn;
        if received_new {
            Ok(Some(self.rx_packet_counter))
        } else {
            self.rx_packet_counter
                .checked_sub(1)
                .map(Some)
                .ok_or_else(|| {
                    "encrypted BLE retransmission precedes packet counter zero".to_owned()
                })
        }
    }

    pub(super) fn observe_security_ecb(
        &mut self,
        input: [u8; 16],
        output: [u8; 16],
    ) -> Result<bool, String> {
        let Some(skd) = self.encryption_skd else {
            return Ok(false);
        };
        if self.encryption_phase == C6BleEncryptionPhase::EncReqReceived {
            let mut reversed_skdm = skd[..8].to_vec();
            reversed_skdm.reverse();
            if input[8..] != reversed_skdm {
                return Ok(false);
            }
            if self.session_key.is_some() {
                return Err("duplicate BLE session-key ECB before LL_ENC_RSP".to_owned());
            }
            let mut firmware_skd = input;
            firmware_skd.reverse();
            self.encryption_skd = Some(firmware_skd);
            self.session_key = Some(output);
            return Ok(true);
        }
        let mut expected_input = skd;
        expected_input.reverse();
        if input != expected_input {
            return Ok(false);
        }
        if self.encryption_phase != C6BleEncryptionPhase::EncRspSent || self.session_key.is_some() {
            return Err(format!(
                "BLE session-key ECB completed during {:?}",
                self.encryption_phase
            ));
        }
        self.session_key = Some(output);
        self.encryption_phase = C6BleEncryptionPhase::SessionKeyReady;
        Ok(true)
    }

    pub(super) fn observe_native_ccm(
        &mut self,
        decrypt: bool,
        peripheral_to_central: bool,
        key: &[u8; 16],
        iv: &[u8; 8],
        packet_counter: u64,
    ) -> Result<bool, String> {
        if self.session_key.as_ref() != Some(key) || self.encryption_iv.as_ref() != Some(iv) {
            return Ok(false);
        }
        if decrypt {
            if peripheral_to_central {
                return Err("native BLE RX CCM used the peripheral-to-central direction".to_owned());
            }
            let expected_counter = self.pending_native_rx_counter.ok_or_else(|| {
                "native BLE RX CCM ran without a pending encrypted reception".to_owned()
            })?;
            if packet_counter != expected_counter {
                return Err(format!(
                    "native BLE RX CCM counter {packet_counter} differs from pending counter {expected_counter}"
                ));
            }
            self.pending_native_rx_counter = None;
        } else {
            if !peripheral_to_central {
                return Err("native BLE TX CCM used the central-to-peripheral direction".to_owned());
            }
            if packet_counter != self.tx_packet_counter {
                return Err(format!(
                    "native BLE TX CCM counter {packet_counter} differs from link counter {}",
                    self.tx_packet_counter
                ));
            }
        }
        Ok(true)
    }

    fn observe_received_control(&mut self, received: &[u8]) -> Result<(), String> {
        if received.first().is_none_or(|header| header & 3 != 3) || received.len() < 3 {
            return Ok(());
        }
        match received[2] {
            0x03 => {
                if received.len() != 25
                    || self.encryption_phase != C6BleEncryptionPhase::Unencrypted
                {
                    return Err(format!(
                        "LL_ENC_REQ length {} received during {:?}",
                        received.len(),
                        self.encryption_phase
                    ));
                }
                let mut skd = [0_u8; 16];
                skd[..8].copy_from_slice(&received[13..21]);
                let mut iv = [0_u8; 8];
                iv[..4].copy_from_slice(&received[21..25]);
                self.encryption_skd = Some(skd);
                self.encryption_iv = Some(iv);
                self.rx_packet_counter = 0;
                self.tx_packet_counter = 0;
                self.encryption_phase = C6BleEncryptionPhase::EncReqReceived;
            }
            0x06 => {
                if received.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::StartReqSent
                {
                    return Err(format!(
                        "LL_START_ENC_RSP received during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::StartRspReceived;
            }
            0x04 | 0x05 => {
                return Err(format!(
                    "central sent peripheral-role encryption opcode {:#04x}",
                    received[2]
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn observe_firmware_control(&mut self, response: &[u8]) -> Result<(), String> {
        if response.first().is_none_or(|header| header & 3 != 3) || response.len() < 3 {
            return Ok(());
        }
        match response[2] {
            0x04 => {
                if response.len() != 15
                    || self.encryption_phase != C6BleEncryptionPhase::EncReqReceived
                {
                    return Err(format!(
                        "LL_ENC_RSP length {} emitted during {:?}",
                        response.len(),
                        self.encryption_phase
                    ));
                }
                let skd = self
                    .encryption_skd
                    .as_mut()
                    .expect("LL_ENC_REQ established SKD storage");
                if self.session_key.is_some() && skd[8..] != response[3..11] {
                    return Err(
                        "LL_ENC_RSP SKDs differs from firmware-programmed ECB input".to_owned()
                    );
                }
                skd[8..].copy_from_slice(&response[3..11]);
                let iv = self
                    .encryption_iv
                    .as_mut()
                    .expect("LL_ENC_REQ established IV storage");
                iv[4..].copy_from_slice(&response[11..15]);
                self.encryption_phase = if self.session_key.is_some() {
                    C6BleEncryptionPhase::SessionKeyReady
                } else {
                    C6BleEncryptionPhase::EncRspSent
                };
            }
            0x05 => {
                if response.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::SessionKeyReady
                {
                    return Err(format!(
                        "LL_START_ENC_REQ emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::StartReqSent;
            }
            0x06 => {
                if response.len() != 3
                    || self.encryption_phase != C6BleEncryptionPhase::StartRspReceived
                {
                    return Err(format!(
                        "LL_START_ENC_RSP emitted during {:?}",
                        self.encryption_phase
                    ));
                }
                self.encryption_phase = C6BleEncryptionPhase::Encrypted;
            }
            0x03 => {
                return Err("peripheral firmware emitted central-role LL_ENC_REQ".to_owned());
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn peripheral_response(
        &mut self,
        received: &[u8],
        firmware_response: Option<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>, String> {
        // Validation decrypts a private copy. The native RX DMA retains the
        // over-air ciphertext so the genuine controller executes its own CCM
        // path through the modeled modem-security ECB peripheral.
        let native_rx_counter = self.native_rx_packet_counter(received)?;
        let decoded_received = self.decode_received(received)?;
        self.pending_native_rx_counter = native_rx_counter;
        let received = decoded_received.as_slice();
        let received_header = received.first().copied().unwrap_or_default();
        let received_sn = received_header & (1 << 3) != 0;
        let received_nesn = received_header & (1 << 2) != 0;

        let mut acknowledged_tx = false;
        if self.awaiting_tx_ack && received_nesn != self.tx_sn {
            if self.last_tx_encrypted {
                self.tx_packet_counter = self
                    .tx_packet_counter
                    .checked_add(1)
                    .ok_or_else(|| "BLE TX packet counter overflow".to_owned())?;
            }
            self.awaiting_tx_ack = false;
            self.tx_sn = !self.tx_sn;
            self.last_tx = None;
            self.last_tx_encrypted = false;
            acknowledged_tx = true;
        }
        let received_new = received_sn == self.expected_rx_sn;
        if received_new {
            let received_encrypted = self.rx_encryption_active();
            self.expected_rx_sn = !self.expected_rx_sn;
            self.observe_received_control(received)?;
            if received_header & 3 == 3
                && received.len() >= 7
                && received.get(1).copied() == Some(5)
                && received.get(2).copied() == Some(0x18)
            {
                let central_tx = received[3];
                let central_rx = received[4];
                let instant = u16::from_le_bytes([received[5], received[6]]);
                let select_phy = |requested, current| match requested {
                    0 => Some(current),
                    1 => Some("ble-1m"),
                    2 => Some("ble-2m"),
                    3 => Some("ble-coded"),
                    _ => None,
                };
                let Some(rx_phy) = select_phy(central_tx, self.rx_phy) else {
                    return Err(format!("invalid central TX PHY value {central_tx}"));
                };
                let Some(tx_phy) = select_phy(central_rx, self.tx_phy) else {
                    return Err(format!("invalid central RX PHY value {central_rx}"));
                };
                let instant_delta = instant.wrapping_sub(self.active_event);
                if !(6..0x8000).contains(&instant_delta) {
                    return Err(format!(
                        "BLE PHY update instant {instant} is {instant_delta} events after current event {}",
                        self.active_event
                    ));
                }
                if self.pending_phy_update.is_some() {
                    return Err("overlapping BLE PHY update procedures".to_owned());
                }
                self.pending_phy_update = Some(C6BlePendingPhyUpdate {
                    instant,
                    tx_phy,
                    rx_phy,
                });
            }
            if received_encrypted {
                self.rx_packet_counter = self
                    .rx_packet_counter
                    .checked_add(1)
                    .ok_or_else(|| "BLE RX packet counter overflow".to_owned())?;
            }
        }

        let hardware_start_response =
            acknowledged_tx && self.encryption_phase == C6BleEncryptionPhase::StartRspReceived;
        let stale_firmware_start_response = acknowledged_tx
            && self.encryption_phase == C6BleEncryptionPhase::Encrypted
            && firmware_response
                .as_ref()
                .is_some_and(|pdu| pdu.get(2) == Some(&0x06));
        let mut response = if self.awaiting_tx_ack {
            self.last_tx.clone()
        } else if hardware_start_response {
            // C6 baseband completes the encryption-start exchange within the
            // continued event, before task-context firmware can advance its
            // TX list from LL_START_ENC_REQ.
            Some(vec![3, 1, 0x06])
        } else if stale_firmware_start_response {
            // The next event sees the just-acknowledged control allocation
            // until recycle advances it. Hardware emits the required empty
            // encrypted acknowledgement instead of retransmitting it.
            Some(vec![1, 0])
        } else {
            firmware_response.or_else(|| Some(vec![1, 0]))
        };
        let retransmission = self.awaiting_tx_ack;
        if let Some(pdu) = response.as_mut()
            && pdu.len() >= 2
        {
            let response_was_encrypted = !retransmission && self.tx_encryption_active();
            if !retransmission {
                // Native TX DMA is plaintext. The baseband applies CCM only
                // after firmware control processing and header sequencing.
                self.observe_firmware_control(pdu)?;
            }
            // LLID and MD come from the firmware buffer (LLID=1 for the
            // hardware-synthesized empty PDU). NESN acknowledges the next
            // expected central SN, while SN remains stable until the central
            // acknowledges this peripheral PDU.
            pdu[0] = (pdu[0] & !0x0c)
                | (u8::from(self.expected_rx_sn) << 2)
                | (u8::from(self.tx_sn) << 3);
            if response_was_encrypted {
                let (key, iv) = self.encryption_material()?;
                *pdu = ble_link_encrypt_pdu(
                    &key,
                    &iv,
                    self.tx_packet_counter,
                    BleLinkDirection::PeripheralToCentral,
                    pdu,
                )
                .map_err(|error| {
                    format!(
                        "native BLE TX CCM counter {} failed: {error}",
                        self.tx_packet_counter
                    )
                })?;
                self.last_tx_encrypted = true;
            }
            self.awaiting_tx_ack = true;
            self.last_tx = Some(pdu.clone());
        }
        Ok(response)
    }
}
