use super::{XtensaMachine, XtensaMachineError};
use remu_core::{AccessKind, AccessWidth, Bus, SimDuration};
use remu_radio::{
    BleController, CoexistenceDecision, CoexistenceRequest, DeliveryOutcome, FrameOrigin,
    MediumEvent, NodeId, RadioDmaDirection, RadioFrame, RadioProtocol, RadioSubsystem, Receiver,
    ReplayArtifact, Spectrum, TxRequest, WifiEngine,
};
const EMULATED_NODE: NodeId = NodeId(1);
const HOST_NODE: NodeId = NodeId(0);
// Native RWBLE interrupt causes, recovered from the revision-zero ROM ISR's
// register dispatch. These are hardware status bits, not symbol hooks: bit 5
// dispatches the programmed-slot END handler and bit 6 the SKIP handler. Bits
// 1 and 2 dispatch TX and RX respectively; bit 18 updates the RX-buffer ring.
const S3_RWBLE_RX_INTERRUPT: u32 = 1 << 2;
const S3_RWBLE_END_INTERRUPT: u32 = 1 << 5;
const S3_RWBLE_SKIP_INTERRUPT: u32 = 1 << 6;
const S3_BLE_INTERFRAME_SPACE_TICKS: u64 = 2_400;
const S3_BLE_FINE_POSITION_TICKS: u64 = 8;
const S3_BLE_FINE_POSITIONS_PER_HALF_SLOT: u64 = 625;
const S3_BLE_HALF_SLOT_TICKS: u64 =
    S3_BLE_FINE_POSITION_TICKS * S3_BLE_FINE_POSITIONS_PER_HALF_SLOT;
const S3_BLE_COARSE_MASK: u64 = 0x0fff_ffff;
const S3_BLE_CLOCK_CYCLE_TICKS: u64 = (S3_BLE_COARSE_MASK + 1) * S3_BLE_HALF_SLOT_TICKS;
const BLE_ADVERTISING_ACCESS_ADDRESS: u32 = 0x8e89_bed6;

pub(super) struct PendingNativeBleTransmission {
    start: u64,
    slot_address: u32,
    channel: u8,
    pdu: Vec<u8>,
}

pub(super) struct PendingNativeBleReception {
    start: u64,
    end: u64,
    slot_address: u32,
    event_index: u8,
    channel: u8,
}

impl XtensaMachine {
    /// Returns the S3 functional Wi-Fi engine when its clock/reset domain is ready.
    pub fn wifi_engine(&mut self) -> Result<&mut WifiEngine, XtensaMachineError> {
        if !self.syscon.wifi_ready() {
            return Err(XtensaMachineError::RadioNotReady("Wi-Fi"));
        }
        Ok(&mut self.radio_wifi)
    }

    /// Returns the S3 functional BLE HCI controller when its domain is ready.
    pub fn ble_controller(&mut self) -> Result<&mut BleController, XtensaMachineError> {
        if !self.syscon.ble_ready() {
            return Err(XtensaMachineError::RadioNotReady("Bluetooth LE"));
        }
        Ok(&mut self.radio_ble)
    }

    /// Injects one explicit packet into the host-isolated deterministic medium.
    pub fn inject_radio_frame(
        &mut self,
        protocol: RadioProtocol,
        spectrum: Spectrum,
        phy: impl Into<String>,
        bytes: Vec<u8>,
        power_dbm: i16,
    ) -> Result<(), XtensaMachineError> {
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
    ) -> Result<(), XtensaMachineError> {
        self.radio_medium.tune_receiver(Receiver {
            node: EMULATED_NODE,
            protocol,
            spectrum,
            sensitivity_dbm: -100,
        })?;
        let duration = frame_duration(bytes.len());
        self.radio_medium.transmit(TxRequest {
            source: HOST_NODE,
            start: at,
            end: at
                .checked_add(duration)
                .map_err(|_| XtensaMachineError::TimeOverflow)?,
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

    /// Returns a versioned snapshot of S3 RF and coexistence events.
    pub fn radio_replay_artifact(&self) -> ReplayArtifact {
        ReplayArtifact::new(
            self.radio_medium.profile().clone(),
            self.radio_medium.events().to_vec(),
        )
        .with_coexistence_events(self.radio_coexistence.events().to_vec())
    }

    pub(super) fn service_radio(&mut self) -> Result<u64, XtensaMachineError> {
        let wifi_ready = self.syscon.wifi_ready();
        let ble_ready = self.syscon.ble_ready();
        let coexistence_ready =
            self.syscon.radio_clock_enable() != 0 && self.syscon.radio_reset_enable() == 0;
        self.radio_legality
            .observe_domain(RadioSubsystem::Wifi, wifi_ready, None, self.now)?;
        self.radio_legality.observe_domain(
            RadioSubsystem::BluetoothLe,
            ble_ready,
            None,
            self.now,
        )?;
        self.radio_legality.observe_domain(
            RadioSubsystem::Coexistence,
            coexistence_ready,
            None,
            self.now,
        )?;
        self.radio_medium.advance_to(self.now)?;
        self.radio_coexistence.advance_to(self.now)?;
        self.complete_native_ble_slot_states();
        self.ble_exchange_memory.advance_to(self.now);
        if self.syscon.ble_ready() {
            self.radio_ble.advance_to(self.now);
        }
        if self.syscon.wifi_ready() {
            self.radio_wifi.advance_to(self.now);
        }
        let mut events = self.complete_radio_receptions()?;
        events = events.saturating_add(self.submit_native_wifi_frames()?);
        events = events.saturating_add(self.submit_native_ble_frames()?);
        events = events.saturating_add(self.service_pending_native_ble_frames()?);
        events = events.saturating_add(self.submit_protocol_frames()?);
        let wifi_pending = self.radio_wifi.has_rx() || self.wifi_mac.interrupt_pending();
        let ble_pending = self.radio_ble.has_h4_output();
        let rwble_pending = self.ble_exchange_memory.interrupt_pending();
        self.radio_legality
            .observe_interrupt(RadioSubsystem::Wifi, wifi_pending, self.now)?;
        self.radio_legality.observe_interrupt(
            RadioSubsystem::BluetoothLe,
            ble_pending || rwble_pending,
            self.now,
        )?;
        self.update_matrix_source(0, wifi_pending)?;
        self.update_matrix_source(4, ble_pending)?;
        self.update_matrix_source(8, rwble_pending)?;
        Ok(events)
    }

    fn submit_native_ble_frames(&mut self) -> Result<u64, XtensaMachineError> {
        while let Some(kick) = self.ble_exchange_memory.take_schedule_kick() {
            let slot = (kick.control & 0x0f) as u16;
            let Some(slot_address) = self
                .ble_exchange_memory
                .resolve_em_address(slot.saturating_mul(16))
            else {
                continue;
            };
            let Some(cs_reference) = self.read_native_ble_u16(slot_address.wrapping_add(8)) else {
                continue;
            };
            let Some(coarse_low) = self.read_native_ble_u16(slot_address.wrapping_add(2)) else {
                continue;
            };
            let Some(coarse_high) = self.read_native_ble_u16(slot_address.wrapping_add(4)) else {
                continue;
            };
            let Some(fine) = self.read_native_ble_u16(slot_address.wrapping_add(6)) else {
                continue;
            };
            let coarse =
                (u64::from(coarse_low) | (u64::from(coarse_high) << 16)) & S3_BLE_COARSE_MASK;
            let tx_start = self.native_ble_slot_time(coarse, u64::from(fine));
            let Some(cs_address) = self
                .ble_exchange_memory
                .resolve_em_address(cs_reference.saturating_mul(2))
            else {
                continue;
            };
            let Some(access_address) = self.read_native_ble_u32(cs_address.wrapping_add(12)) else {
                continue;
            };
            let Some(event_word) = self.read_native_ble_u16(cs_address.wrapping_add(2)) else {
                continue;
            };
            let event_index = s3_ble_event_index(event_word);
            let Some(channel_word) = self.read_native_ble_u16(cs_address.wrapping_add(22)) else {
                continue;
            };
            let channel = (channel_word & 0x3f) as u8;
            let Some(tx_descriptor_offset) = self.read_native_ble_u16(cs_address.wrapping_add(28))
            else {
                continue;
            };
            if tx_descriptor_offset == 0 {
                // A receive-only scan control structure has no TX descriptor.
                // Its window is expressed in 0.625-ms BLE slots at CS+0x20.
                // Record the actual RF aperture and complete the scheduler
                // event at the programmed end rather than interpreting the
                // receive control block as a packet descriptor.
                let Some(window_units) = self.read_native_ble_u16(cs_address.wrapping_add(32))
                else {
                    continue;
                };
                let window_ticks = u64::from(window_units.max(1))
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
                    },
                );
                self.ble_exchange_memory
                    .schedule_radio_completion(end, S3_RWBLE_END_INTERRUPT);
                self.schedule_native_ble_slot_state(end, slot_address, 4);
                continue;
            }
            let Some(tx_descriptor) = self
                .ble_exchange_memory
                .resolve_em_address(tx_descriptor_offset)
            else {
                continue;
            };
            let Some(header) = self.read_native_ble_u16(tx_descriptor.wrapping_add(2)) else {
                continue;
            };
            let Some(payload_offset) = self.read_native_ble_u16(tx_descriptor.wrapping_add(4))
            else {
                continue;
            };
            let Some(payload_address) = self.ble_exchange_memory.resolve_em_address(payload_offset)
            else {
                continue;
            };
            let header_byte = header as u8;
            let declared_length = usize::from((header >> 8) as u8);
            let advertising_pdu_with_local_address = access_address
                == BLE_ADVERTISING_ACCESS_ADDRESS
                && matches!(header_byte & 0x0f, 0 | 1 | 2 | 4 | 6);
            let payload_length = if advertising_pdu_with_local_address {
                let Some(length) = declared_length.checked_sub(6) else {
                    continue;
                };
                length
            } else {
                declared_length
            };
            let Some(payload) = self.read_native_ble_bytes(payload_address, payload_length) else {
                continue;
            };
            let mut pdu = Vec::with_capacity(2 + declared_length);
            pdu.extend_from_slice(&[header_byte, declared_length as u8]);
            if advertising_pdu_with_local_address {
                let Some(address) = self.native_bluetooth_address() else {
                    continue;
                };
                pdu.extend_from_slice(&address);
            }
            pdu.extend_from_slice(&payload);

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
                    channel,
                    pdu,
                },
            );
        }
        Ok(0)
    }

    fn service_pending_native_ble_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut submitted = 0_u64;
        while self
            .pending_native_ble_transmissions
            .front()
            .is_some_and(|pending| pending.start <= self.now.ticks())
        {
            let Some(pending) = self.pending_native_ble_transmissions.pop_front() else {
                break;
            };
            let duration = frame_duration(pending.pdu.len());
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
            if matches!(decision, CoexistenceDecision::Granted { .. }) {
                self.radio_medium.transmit(TxRequest {
                    source: EMULATED_NODE,
                    start: self.now,
                    end: due,
                    power_dbm: 0,
                    frame: RadioFrame {
                        protocol: RadioProtocol::BluetoothLe,
                        spectrum: s3_ble_spectrum(pending.channel),
                        phy: "ble-1m".to_owned(),
                        bytes: pending.pdu,
                        origin: FrameOrigin::Emulated,
                    },
                })?;
                // A legacy advertising slot does not request a standalone TX
                // callback. Raising RWBLE's global TX cause here would make
                // the scheduler deliver status 3 to lld_adv, which is invalid
                // for this slot type. Hardware reports the completed event via
                // END after the inter-frame response window instead.
                self.schedule_native_ble_slot_state(due, pending.slot_address, 2);
                let end_due = due
                    .checked_add(SimDuration::from_ticks(S3_BLE_INTERFRAME_SPACE_TICKS))
                    .map_err(|_| XtensaMachineError::TimeOverflow)?;
                self.ble_exchange_memory
                    .schedule_radio_completion(end_due, S3_RWBLE_END_INTERRUPT);
                self.schedule_native_ble_slot_state(end_due, pending.slot_address, 4);
                submitted = submitted.saturating_add(1);
            } else {
                self.ble_exchange_memory
                    .schedule_radio_completion(due, S3_RWBLE_SKIP_INTERRUPT);
            }
        }
        Ok(submitted)
    }

    fn native_ble_slot_time(&self, coarse: u64, fine: u64) -> remu_core::SimTime {
        let fine = fine.min(S3_BLE_FINE_POSITIONS_PER_HALF_SLOT - 1);
        let target_in_cycle = coarse * S3_BLE_HALF_SLOT_TICKS
            + (S3_BLE_FINE_POSITIONS_PER_HALF_SLOT - 1 - fine) * S3_BLE_FINE_POSITION_TICKS;
        let now_in_cycle = self.now.ticks() % S3_BLE_CLOCK_CYCLE_TICKS;
        let delta = target_in_cycle
            .wrapping_add(S3_BLE_CLOCK_CYCLE_TICKS)
            .wrapping_sub(now_in_cycle)
            % S3_BLE_CLOCK_CYCLE_TICKS;
        remu_core::SimTime::from_ticks(self.now.ticks().saturating_add(delta))
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

    fn complete_native_ble_slot_states(&mut self) {
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
            let Some(control) = self.read_native_ble_u16(slot_address) else {
                continue;
            };
            // RWBLE owns event-table state bits 3:5 after firmware starts a
            // slot. State 2 denotes the completed frame visible to the TX ISR;
            // state 4 denotes successful event completion for the END ISR.
            // Firmware owns all remaining command fields.
            let completed = (control & !0x0038) | (state << 3);
            let _ = self.bus.write(
                u64::from(slot_address),
                AccessWidth::HalfWord,
                u64::from(completed),
                self.now,
            );
        }
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

    fn native_bluetooth_address(&mut self) -> Option<[u8; 6]> {
        let low = self
            .bus
            .read(0x6000_7044, AccessWidth::Word, AccessKind::Read, self.now)
            .ok()? as u32;
        let high = self
            .bus
            .read(0x6000_7048, AccessWidth::Word, AccessKind::Read, self.now)
            .ok()? as u32;
        let mut address = [0_u8; 6];
        address[..4].copy_from_slice(&low.to_le_bytes());
        address[4..].copy_from_slice(&high.to_le_bytes()[..2]);
        // ESP32-S3 derives the Bluetooth universal address from the factory
        // base address by adding two to the least-significant octet.
        address[0] = address[0].wrapping_add(2);
        Some(address)
    }

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
            let length = ((control as usize) >> 12) & 0x0fff;
            self.radio_legality.require(
                RadioSubsystem::Wifi,
                remu_radio::RadioLegalityRule::DmaLength,
                length <= capacity,
                self.now,
                format!("TX DMA length {length} exceeds descriptor capacity {capacity}"),
            )?;
            self.radio_legality.validate_dma(
                RadioSubsystem::Wifi,
                RadioDmaDirection::Transmit,
                buffer as u32,
                4,
                length,
                4095,
                self.now,
            )?;
            let bytes = (0..length)
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
            let duration = frame_duration(bytes.len());
            self.radio_medium.transmit(TxRequest {
                source: EMULATED_NODE,
                start: self.now,
                end: self
                    .now
                    .checked_add(duration)
                    .map_err(|_| XtensaMachineError::TimeOverflow)?,
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

    fn complete_radio_receptions(&mut self) -> Result<u64, XtensaMachineError> {
        let new_events = &self.radio_medium.events()[self.radio_event_cursor..];
        let mut deliveries = Vec::new();
        for event in new_events {
            let MediumEvent::Reception {
                id,
                receiver: EMULATED_NODE,
                outcome: DeliveryOutcome::Delivered,
            } = event
            else {
                continue;
            };
            if let Some((frame, signal_dbm)) =
                self.radio_medium
                    .events()
                    .iter()
                    .find_map(|candidate| match candidate {
                        MediumEvent::Submitted {
                            id: candidate_id,
                            request,
                        } if candidate_id == id => Some((
                            request.frame.clone(),
                            self.radio_medium.received_power_dbm(
                                request.source,
                                EMULATED_NODE,
                                request.power_dbm,
                            ),
                        )),
                        _ => None,
                    })
            {
                deliveries.push((frame, signal_dbm));
            }
        }
        self.radio_event_cursor = self.radio_medium.events().len();
        let mut completed = 0_u64;
        for (frame, signal_dbm) in deliveries {
            match frame.protocol {
                RadioProtocol::Wifi => {
                    let wifi_mac = self.wifi_mac.clone();
                    let native = self.write_native_wifi_rx(&wifi_mac, &frame.bytes);
                    if self.syscon.wifi_ready() && self.radio_wifi.receive(&frame).unwrap_or(false)
                    {
                        completed = completed.saturating_add(1);
                    } else if native {
                        completed = completed.saturating_add(1);
                    }
                }
                RadioProtocol::BluetoothLe if self.syscon.ble_ready() => {
                    let native = self.write_native_ble_rx(&frame, signal_dbm);
                    if self
                        .radio_ble
                        .receive_rf(
                            &frame,
                            signal_dbm.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8,
                        )
                        .unwrap_or(false)
                    {
                        completed = completed.saturating_add(1);
                    } else if native {
                        completed = completed.saturating_add(1);
                    }
                }
                RadioProtocol::BluetoothLe | RadioProtocol::Ieee802154 => {}
            }
        }
        Ok(completed)
    }

    fn write_native_ble_rx(&mut self, frame: &RadioFrame, signal_dbm: i16) -> bool {
        self.pending_native_ble_receptions
            .retain(|pending| pending.end >= self.now.ticks());
        let Some(activity) = self
            .pending_native_ble_receptions
            .iter()
            .find(|pending| {
                pending.start <= self.now.ticks()
                    && pending.end >= self.now.ticks()
                    && frame.spectrum.overlaps(s3_ble_spectrum(pending.channel))
            })
            .map(|pending| (pending.slot_address, pending.event_index, pending.channel))
        else {
            return false;
        };
        if frame.bytes.len() < 2 {
            return false;
        }

        let current = self.ble_exchange_memory.rx_buffer_current() & 0x7fff;
        let Some(descriptor) = self.ble_exchange_memory.resolve_em_address(current) else {
            return false;
        };
        let Some(status) = self.read_native_ble_u16(descriptor.wrapping_add(2)) else {
            return false;
        };
        if status & 0x8000 == 0 {
            return false;
        }
        let Some(buffer_offset) = self.read_native_ble_u16(descriptor.wrapping_add(18)) else {
            return false;
        };
        let Some(buffer) = self.ble_exchange_memory.resolve_em_address(buffer_offset) else {
            return false;
        };
        let Some(next) = self.read_native_ble_u16(descriptor) else {
            return false;
        };
        let header = u16::from_le_bytes([frame.bytes[0], frame.bytes[1]]);
        let coarse = ((self.now.ticks() / S3_BLE_HALF_SLOT_TICKS) & S3_BLE_COARSE_MASK) as u32;
        let fine =
            ((self.now.ticks() % S3_BLE_HALF_SLOT_TICKS) / S3_BLE_FINE_POSITION_TICKS) as u16;
        // The revision-zero ROM reads the low byte at +6 as signed RXRSSI and
        // the low six bits at +14 as RXCHASS. Keep the intervening timestamp
        // fields in their recovered order; +12 carries the scheduler event
        // index used by r_lld_rxdesc_check.
        let raw_rssi = signal_dbm.clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8 as u8;
        let metadata = [
            u16::from(raw_rssi),
            (coarse & 0xffff) as u16,
            fine,
            u16::from(activity.1 & 0x1f) << 11,
            u16::from(activity.2),
            0,
        ];
        if !self.write_native_ble_bytes(buffer, &frame.bytes[2..])
            || !self.write_native_ble_u16(descriptor, next | 0x8000)
            || !self.write_native_ble_u16(descriptor.wrapping_add(4), header)
            || metadata.iter().enumerate().any(|(index, value)| {
                !self.write_native_ble_u16(descriptor.wrapping_add(6 + (index as u32 * 2)), *value)
            })
            || !self.write_native_ble_u16(descriptor.wrapping_add(2), status & !0x8000)
        {
            return false;
        }
        self.ble_exchange_memory.advance_rx_buffer(next & 0x7fff);
        self.schedule_native_ble_slot_state(self.now, activity.0, 2);
        self.ble_exchange_memory
            .raise_interrupt(S3_RWBLE_RX_INTERRUPT);
        true
    }

    fn write_native_ble_u16(&mut self, address: u32, value: u16) -> bool {
        self.bus
            .write(
                u64::from(address),
                AccessWidth::HalfWord,
                u64::from(value),
                self.now,
            )
            .is_ok()
    }

    fn write_native_ble_bytes(&mut self, address: u32, bytes: &[u8]) -> bool {
        bytes.iter().enumerate().all(|(offset, byte)| {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .is_ok()
        })
    }

    fn write_native_wifi_rx(
        &mut self,
        wifi_mac: &remu_devices::Esp32S3WifiMacHandle,
        frame: &[u8],
    ) -> bool {
        let Some(descriptor) = wifi_mac.rx_descriptor() else {
            return false;
        };
        let Ok(control) = self.bus.read(
            u64::from(descriptor.address),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        if control as u32 & (1 << 31) == 0 {
            return false;
        }
        let Ok(buffer) = self.bus.read(
            u64::from(descriptor.address.wrapping_add(4)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        let Ok(next) = self.bus.read(
            u64::from(descriptor.address.wrapping_add(8)),
            AccessWidth::Word,
            AccessKind::Read,
            self.now,
        ) else {
            return false;
        };
        let capacity = (control as usize) & 0x0fff;
        let metadata = s3_wifi_rx_metadata(frame.len(), self.now);
        let total = metadata.len().saturating_add(frame.len()).saturating_add(4);
        if buffer == 0 || total > capacity || total > 0x0fff {
            return false;
        }
        if !self.write_native_wifi_bytes(buffer as u32, &metadata)
            || !self
                .write_native_wifi_bytes((buffer as u32).wrapping_add(metadata.len() as u32), frame)
            || !self.write_native_wifi_bytes(
                (buffer as u32).wrapping_add((metadata.len() + frame.len()) as u32),
                &[0; 4],
            )
            || self
                .bus
                .write(
                    u64::from(descriptor.address),
                    AccessWidth::Word,
                    u64::from((control as u32 & 0x0000_0fff) | ((total as u32) << 12) | (1 << 30)),
                    self.now,
                )
                .is_err()
        {
            return false;
        }
        wifi_mac.complete_rx_descriptor(descriptor.address, next as u32);
        true
    }

    fn write_native_wifi_bytes(&mut self, address: u32, bytes: &[u8]) -> bool {
        bytes.iter().enumerate().all(|(offset, byte)| {
            self.bus
                .write(
                    u64::from(address.wrapping_add(offset as u32)),
                    AccessWidth::Byte,
                    u64::from(*byte),
                    self.now,
                )
                .is_ok()
        })
    }

    fn submit_protocol_frames(&mut self) -> Result<u64, XtensaMachineError> {
        let mut frames = Vec::new();
        if self.syscon.wifi_ready() {
            while let Some((_, frame)) = self.radio_wifi.take_tx() {
                frames.push((frame, 8));
            }
        }
        if self.syscon.ble_ready() {
            while let Some(frame) = self.radio_ble.take_rf_output() {
                frames.push((frame, 9));
            }
        }
        let mut submitted = 0_u64;
        for (frame, priority) in frames {
            let duration = frame_duration(frame.bytes.len());
            let decision = self.radio_coexistence.request(CoexistenceRequest {
                protocol: frame.protocol,
                start: self.now,
                duration,
                priority,
                preemptible: true,
            })?;
            if !matches!(decision, CoexistenceDecision::Granted { .. }) {
                continue;
            }
            self.radio_medium.transmit(TxRequest {
                source: EMULATED_NODE,
                start: self.now,
                end: self
                    .now
                    .checked_add(duration)
                    .map_err(|_| XtensaMachineError::TimeOverflow)?,
                power_dbm: 0,
                frame,
            })?;
            submitted = submitted.saturating_add(1);
        }
        Ok(submitted)
    }
}

fn frame_duration(length: usize) -> SimDuration {
    SimDuration::from_ticks(
        u64::try_from(length.max(1))
            .unwrap_or(u64::MAX)
            .saturating_mul(8),
    )
}

fn s3_ble_spectrum(channel: u8) -> Spectrum {
    let center_khz = match channel {
        37 => 2_402_000,
        38 => 2_426_000,
        39 => 2_480_000,
        0..=10 => 2_404_000 + u32::from(channel) * 2_000,
        11..=36 => 2_428_000 + u32::from(channel - 11) * 2_000,
        _ => 2_402_000,
    };
    Spectrum::new(center_khz, 2_000)
}

// The native control structure retains the controller event identity in the
// low five bits of CS+2. Scheduler table slots are recycled independently, so
// their rolling slot number cannot be used in RX descriptor metadata.
fn s3_ble_event_index(control: u16) -> u8 {
    (control & 0x1f) as u8
}

fn s3_wifi_rx_metadata(frame_length: usize, at: remu_core::SimTime) -> [u8; 48] {
    let mut metadata = [0_u8; 48];
    metadata[0] = (-40_i8) as u8;
    metadata[3] = 1 << 4;
    metadata[8] = 1 << 1;
    metadata[11] = (1 << 7) | 1;
    metadata[12..16].copy_from_slice(&(at.ticks() as u32).to_le_bytes());
    metadata[20] = (-95_i8) as u8;
    let signal_length = frame_length.saturating_add(4).min(0x0fff) as u32;
    metadata[44..48].copy_from_slice(&signal_length.to_le_bytes());
    metadata
}

#[cfg(test)]
mod tests {
    use super::s3_ble_event_index;

    #[test]
    fn ble_event_index_comes_from_control_structure_not_scheduler_slot() {
        assert_eq!(s3_ble_event_index(0x0880), 0);
        assert_eq!(s3_ble_event_index(0x0881), 1);
        assert_eq!(s3_ble_event_index(0xffff), 31);
    }
}
