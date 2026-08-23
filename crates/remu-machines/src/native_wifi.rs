use remu_core::{SimDuration, SimTime};

/// One hardware-owned native Wi-Fi transmit operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingNativeWifiTransmission {
    pub(crate) queue: u8,
    pub(crate) end: SimTime,
    pub(crate) deadline: SimTime,
    pub(crate) expected_response: Option<NativeWifiResponse>,
    response_timeout: u64,
    protected_payload: Option<Vec<Vec<u8>>>,
    block_ack_sequences: Vec<u16>,
}

/// Control response that completes one native transmit exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeWifiResponse {
    Ack { receiver: [u8; 6] },
    Cts { receiver: [u8; 6] },
    BlockAck { receiver: [u8; 6], tid: u8 },
}

/// Decoded compressed block-ACK request delivered to the native MAC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeWifiBlockAckRequest {
    pub(crate) receiver: [u8; 6],
    pub(crate) transmitter: [u8; 6],
    pub(crate) tid: u8,
    pub(crate) starting_sequence: u16,
}

/// QoS receive metadata consumed by the hardware RX block-ACK engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeWifiQosMpdu {
    pub(crate) receiver: [u8; 6],
    pub(crate) transmitter: [u8; 6],
    pub(crate) tid: u8,
    pub(crate) sequence: u16,
}

/// Firmware-observed invariants shared by TX and RX native A-MPDU handling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeWifiAggregate {
    pub(crate) receiver: [u8; 6],
    pub(crate) transmitter: [u8; 6],
    pub(crate) tid: u8,
    pub(crate) sequences: Vec<u16>,
}

impl PendingNativeWifiTransmission {
    /// Builds the completion window described by the outgoing MAC header and
    /// the guest-programmed per-queue timeout register.
    pub(crate) fn new(
        queue: u8,
        transmitted_frame: &[u8],
        protected_payload: Option<Vec<u8>>,
        end: SimTime,
        programmed_timeout: u16,
    ) -> Option<Self> {
        let expected_response = response_for_transmit(transmitted_frame);
        let block_ack_sequences =
            if matches!(expected_response, Some(NativeWifiResponse::BlockAck { .. })) {
                native_wifi_qos_mpdu(transmitted_frame)
                    .map(|mpdu| vec![mpdu.sequence])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
        // Bare-metal probes may leave the timeout field at reset. Hardware
        // still has a finite ACK window, so retain a small deterministic
        // fallback instead of completing an ACK-required frame immediately.
        let timeout = u64::from(programmed_timeout).max(256);
        let deadline = if expected_response.is_some() {
            end.checked_add(SimDuration::from_ticks(timeout)).ok()?
        } else {
            end
        };
        Some(Self {
            queue,
            end,
            deadline,
            expected_response,
            response_timeout: timeout,
            protected_payload: protected_payload.map(|frame| vec![frame]),
            block_ack_sequences,
        })
    }

    /// Builds one hardware completion window for a descriptor-linked A-MPDU.
    pub(crate) fn new_aggregate(
        queue: u8,
        transmitted_frames: &[Vec<u8>],
        end: SimTime,
        programmed_timeout: u16,
    ) -> Result<Self, String> {
        Self::new_aggregate_with_minimum(queue, transmitted_frames, end, programmed_timeout, 2)
    }

    /// Builds a completion window for the S3 hardware A-MPDU record.
    ///
    /// Genuine S3 LMAC can submit its eight-byte A-MPDU buffer form with a
    /// single MPDU while a BA session is warming up. The explicit hardware
    /// marker, rather than the descriptor count alone, selects this path.
    pub(crate) fn new_s3_hardware_aggregate(
        queue: u8,
        transmitted_frames: &[Vec<u8>],
        end: SimTime,
        programmed_timeout: u16,
    ) -> Result<Self, String> {
        Self::new_aggregate_with_minimum(queue, transmitted_frames, end, programmed_timeout, 1)
    }

    fn new_aggregate_with_minimum(
        queue: u8,
        transmitted_frames: &[Vec<u8>],
        end: SimTime,
        programmed_timeout: u16,
        minimum_mpdus: usize,
    ) -> Result<Self, String> {
        let aggregate =
            validate_native_wifi_aggregate_with_minimum(transmitted_frames, minimum_mpdus)?;
        let expected_response = Some(NativeWifiResponse::BlockAck {
            receiver: aggregate.transmitter,
            tid: aggregate.tid,
        });
        let timeout = u64::from(programmed_timeout).max(256);
        let deadline = end
            .checked_add(SimDuration::from_ticks(timeout))
            .map_err(|_| {
                "native aggregate response deadline overflows simulation time".to_owned()
            })?;
        Ok(Self {
            queue,
            end,
            deadline,
            expected_response,
            response_timeout: timeout,
            protected_payload: None,
            block_ack_sequences: aggregate.sequences,
        })
    }

    pub(crate) fn accepts_response(&self, frame: &[u8], received_at: SimTime) -> bool {
        self.end <= received_at
            && received_at <= self.deadline
            && self.expected_response.is_some_and(|expected| {
                wifi_response(frame).is_some_and(|actual| actual == expected)
            })
    }

    /// Whether this exchange has sent hardware RTS but not its protected MPDU.
    pub(crate) fn awaiting_protection_cts(&self) -> bool {
        self.protected_payload.is_some()
    }

    pub(crate) fn protected_payload_len(&self) -> Option<usize> {
        self.protected_payload
            .as_ref()
            .map(|frames| frames.iter().map(Vec::len).sum())
    }

    /// Moves an RTS-protected exchange into its data/ACK-or-BA phase.
    pub(crate) fn begin_protected_payload(&mut self, end: SimTime) -> Option<Vec<Vec<u8>>> {
        let frames = self.protected_payload.take()?;
        if frames.len() == 1 {
            self.expected_response = response_for_transmit(&frames[0]);
            self.block_ack_sequences = if matches!(
                self.expected_response,
                Some(NativeWifiResponse::BlockAck { .. })
            ) {
                native_wifi_qos_mpdu(&frames[0])
                    .map(|mpdu| vec![mpdu.sequence])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
        } else {
            let aggregate =
                Self::new_aggregate(self.queue, &frames, end, self.response_timeout as u16).ok()?;
            self.expected_response = aggregate.expected_response;
            self.block_ack_sequences = aggregate.block_ack_sequences;
        }
        self.end = end;
        self.deadline = if self.expected_response.is_some() {
            end.checked_add(SimDuration::from_ticks(self.response_timeout))
                .ok()?
        } else {
            end
        };
        Some(frames)
    }

    /// Replaces a single deferred payload with a descriptor-linked aggregate.
    pub(crate) fn protect_aggregate(&mut self, frames: Vec<Vec<u8>>) -> Result<(), String> {
        if !self.awaiting_protection_cts() {
            return Err("native aggregate protection has no active RTS phase".to_owned());
        }
        // Validate the aggregate's eventual BA exchange before any RF work is
        // submitted, but retain the current CTS response window.
        Self::new_aggregate(self.queue, &frames, self.end, self.response_timeout as u16)?;
        self.protected_payload = Some(frames);
        Ok(())
    }

    /// Hardware completion selected when this exchange reaches its deadline.
    pub(crate) fn deadline_outcome(&self) -> remu_devices::EspWifiTxOutcome {
        if self.awaiting_protection_cts() {
            remu_devices::EspWifiTxOutcome::CtsTimeout
        } else if self.expected_response.is_some() {
            remu_devices::EspWifiTxOutcome::AckTimeout
        } else {
            remu_devices::EspWifiTxOutcome::Success
        }
    }

    /// Converts an accepted ACK or BA into the native completion fields.
    pub(crate) fn response_completion(
        &self,
        frame: &[u8],
        received_at: SimTime,
    ) -> Option<(u8, Option<remu_devices::EspWifiTxBlockAck>)> {
        if !self.accepts_response(frame, received_at) {
            return None;
        }
        if !matches!(
            self.expected_response,
            Some(NativeWifiResponse::BlockAck { .. })
        ) {
            return Some((1, None));
        }
        let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
        if frame_control & 0x00fc != 0x0094 {
            return None;
        }
        let (_, compressed) = block_ack_control(frame)?;
        if !compressed {
            return None;
        }
        let starting_sequence =
            u16::from_le_bytes(frame.get(18..20)?.try_into().ok()?) >> 4 & 0x0fff;
        let bitmap = u64::from_le_bytes(frame.get(20..28)?.try_into().ok()?);
        let successful = self
            .block_ack_sequences
            .iter()
            .filter(|sequence| {
                let delta = sequence.wrapping_sub(starting_sequence) & 0x0fff;
                delta < 64 && bitmap & (1_u64 << delta) != 0
            })
            .count() as u8;
        Some((
            successful,
            Some(remu_devices::EspWifiTxBlockAck {
                status: 0,
                starting_sequence,
                bitmap,
            }),
        ))
    }
}

/// Builds the RTS control MPDU generated by a descriptor-protected queue.
///
/// The pinned C6 and S3 HALs name software-descriptor bit eight `rts` and map
/// it to a native per-queue protection bit. Hardware protection is valid only
/// for an individually addressed management or data MPDU that would otherwise
/// require ACK or block ACK. The firmware-computed Duration/ID is retained.
pub(crate) fn native_wifi_rts_protection(frame: &[u8]) -> Option<Vec<u8>> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    if frame_type != 0 && frame_type != 2 {
        return None;
    }
    let receiver: [u8; 6] = frame.get(4..10)?.try_into().ok()?;
    if receiver[0] & 1 != 0 || response_for_transmit(frame).is_none() {
        return None;
    }
    let transmitter: [u8; 6] = frame.get(10..16)?.try_into().ok()?;
    let duration = frame.get(2..4)?;
    let mut rts = vec![0xb4, 0x00, duration[0], duration[1]];
    rts.extend_from_slice(&receiver);
    rts.extend_from_slice(&transmitter);
    Some(rts)
}

/// Returns the control response that the outgoing exchange must receive.
///
/// Management and unicast data frames use address two as the transmitter.
/// RTS and compressed BAR exchanges wait for CTS and block ACK respectively.
/// Group frames and QoS frames with no-ACK/no-explicit-ACK policy complete
/// after airtime without opening a response window.
fn response_for_transmit(frame: &[u8]) -> Option<NativeWifiResponse> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    let subtype = (frame_control >> 4) & 0xf;
    if frame_type == 1 {
        let transmitter: [u8; 6] = frame.get(10..16)?.try_into().ok()?;
        return match subtype {
            0x8 => Some(NativeWifiResponse::BlockAck {
                receiver: transmitter,
                tid: block_ack_control(frame)?.0,
            }),
            0xb => Some(NativeWifiResponse::Cts {
                receiver: transmitter,
            }),
            _ => None,
        };
    }
    if frame_type != 0 && frame_type != 2 {
        return None;
    }
    let receiver: [u8; 6] = frame.get(4..10)?.try_into().ok()?;
    if receiver[0] & 1 != 0 {
        return None;
    }
    let transmitter: [u8; 6] = frame.get(10..16)?.try_into().ok()?;
    if frame_type == 2 && subtype & 0x8 != 0 {
        let has_address_four = frame_control & (3 << 8) == 3 << 8;
        let qos_offset = if has_address_four { 30 } else { 24 };
        let qos_control =
            u16::from_le_bytes(frame.get(qos_offset..qos_offset + 2)?.try_into().ok()?);
        return match qos_control >> 5 & 0x3 {
            0 => Some(NativeWifiResponse::Ack {
                receiver: transmitter,
            }),
            3 => Some(NativeWifiResponse::BlockAck {
                receiver: transmitter,
                tid: qos_control as u8 & 0xf,
            }),
            _ => None,
        };
    }
    Some(NativeWifiResponse::Ack {
        receiver: transmitter,
    })
}

/// Builds the hardware-generated immediate ACK or CTS for an addressed frame.
pub(crate) fn native_wifi_immediate_response(frame: &[u8]) -> Option<Vec<u8>> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    let subtype = (frame_control >> 4) & 0xf;
    let receiver: [u8; 6] = frame.get(4..10)?.try_into().ok()?;
    if receiver[0] & 1 != 0 {
        return None;
    }
    let transmitter: [u8; 6] = frame.get(10..16)?.try_into().ok()?;
    if frame_type == 1 {
        return (subtype == 0xb).then(|| control_response(0xc4, transmitter));
    }
    if frame_type != 0 && frame_type != 2 {
        return None;
    }
    if frame_type == 2 && subtype & 0x8 != 0 {
        let has_address_four = frame_control & (3 << 8) == 3 << 8;
        let qos_offset = if has_address_four { 30 } else { 24 };
        let qos_control =
            u16::from_le_bytes(frame.get(qos_offset..qos_offset + 2)?.try_into().ok()?);
        if qos_control >> 5 & 0x3 != 0 {
            return None;
        }
    }
    Some(control_response(0xd4, transmitter))
}

/// Decodes a compressed single-TID BAR.
pub(crate) fn native_wifi_block_ack_request(frame: &[u8]) -> Option<NativeWifiBlockAckRequest> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    if frame_control & 0x00fc != 0x0084 {
        return None;
    }
    let receiver = frame.get(4..10)?.try_into().ok()?;
    let transmitter = frame.get(10..16)?.try_into().ok()?;
    let (tid, compressed) = block_ack_control(frame)?;
    if !compressed {
        return None;
    }
    let sequence_control = u16::from_le_bytes(frame.get(18..20)?.try_into().ok()?);
    Some(NativeWifiBlockAckRequest {
        receiver,
        transmitter,
        tid,
        starting_sequence: sequence_control >> 4,
    })
}

/// Decodes the fields used by the native RX block-ACK scoreboard.
pub(crate) fn native_wifi_qos_mpdu(frame: &[u8]) -> Option<NativeWifiQosMpdu> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    let subtype = (frame_control >> 4) & 0xf;
    if frame_type != 2 || subtype & 0x8 == 0 {
        return None;
    }
    let receiver = frame.get(4..10)?.try_into().ok()?;
    let transmitter = frame.get(10..16)?.try_into().ok()?;
    let sequence_control = u16::from_le_bytes(frame.get(22..24)?.try_into().ok()?);
    let qos_offset = if frame_control & (3 << 8) == 3 << 8 {
        30
    } else {
        24
    };
    let qos_control = u16::from_le_bytes(frame.get(qos_offset..qos_offset + 2)?.try_into().ok()?);
    Some(NativeWifiQosMpdu {
        receiver,
        transmitter,
        tid: qos_control as u8 & 0xf,
        sequence: sequence_control >> 4,
    })
}

fn native_wifi_qos_ack_policy(frame: &[u8]) -> Option<u8> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    let subtype = (frame_control >> 4) & 0xf;
    if frame_type != 2 || subtype & 0x8 == 0 {
        return None;
    }
    let qos_offset = if frame_control & (3 << 8) == 3 << 8 {
        30
    } else {
        24
    };
    let qos_control = u16::from_le_bytes(frame.get(qos_offset..qos_offset + 2)?.try_into().ok()?);
    Some(((qos_control >> 5) & 0x3) as u8)
}

/// Validates the native MAC invariants for one descriptor-linked A-MPDU.
pub(crate) fn validate_native_wifi_aggregate(
    frames: &[Vec<u8>],
) -> Result<NativeWifiAggregate, String> {
    validate_native_wifi_aggregate_with_minimum(frames, 2)
}

fn validate_native_wifi_aggregate_with_minimum(
    frames: &[Vec<u8>],
    minimum_mpdus: usize,
) -> Result<NativeWifiAggregate, String> {
    if !(minimum_mpdus..=64).contains(&frames.len()) {
        return Err(format!(
            "native aggregate contains {} MPDUs outside the recovered {}..=64 window",
            frames.len(),
            minimum_mpdus
        ));
    }
    let Some(head) = native_wifi_qos_mpdu(&frames[0]) else {
        return Err("native aggregate MPDU 0 is not QoS data".to_owned());
    };
    let head_ack_policy = native_wifi_qos_ack_policy(&frames[0])
        .expect("aggregate head was already validated as QoS data");
    if !matches!(head_ack_policy, 0 | 3) {
        return Err(format!(
            "native aggregate head uses QoS ACK policy {head_ack_policy}, not implicit/explicit block ACK"
        ));
    }
    let mut sequences = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        if frame.len() > 4095 {
            return Err(format!(
                "native aggregate MPDU {index} has {} bytes beyond the recovered 4095-byte DMA limit",
                frame.len()
            ));
        }
        let Some(mpdu) = native_wifi_qos_mpdu(frame) else {
            return Err(format!("native aggregate MPDU {index} is not QoS data"));
        };
        if mpdu.receiver != head.receiver
            || mpdu.transmitter != head.transmitter
            || mpdu.tid != head.tid
        {
            return Err(format!(
                "native aggregate MPDU {index} changes receiver, transmitter, or TID within one BA exchange"
            ));
        }
        if native_wifi_qos_ack_policy(frame) != Some(head_ack_policy) {
            return Err(format!(
                "native aggregate MPDU {index} does not use the head's QoS ACK policy {head_ack_policy}"
            ));
        }
        if sequences.contains(&mpdu.sequence) {
            return Err(format!(
                "native aggregate repeats sequence {:#05x} at MPDU {index}",
                mpdu.sequence
            ));
        }
        sequences.push(mpdu.sequence);
    }
    Ok(NativeWifiAggregate {
        receiver: head.receiver,
        transmitter: head.transmitter,
        tid: head.tid,
        sequences,
    })
}

/// Builds the compressed block-ACK control response owned by the MAC.
pub(crate) fn native_wifi_block_ack_response(
    request: NativeWifiBlockAckRequest,
    bitmap: u64,
) -> Vec<u8> {
    let mut response = vec![0x94, 0x00, 0, 0];
    response.extend_from_slice(&request.transmitter);
    response.extend_from_slice(&request.receiver);
    response.extend_from_slice(&(0x0004 | (u16::from(request.tid) << 12)).to_le_bytes());
    response.extend_from_slice(&(request.starting_sequence << 4).to_le_bytes());
    response.extend_from_slice(&bitmap.to_le_bytes());
    response
}

fn control_response(frame_control: u8, receiver: [u8; 6]) -> Vec<u8> {
    let mut response = vec![frame_control, 0, 0, 0];
    response.extend_from_slice(&receiver);
    response
}

fn block_ack_control(frame: &[u8]) -> Option<(u8, bool)> {
    let control = u16::from_le_bytes(frame.get(16..18)?.try_into().ok()?);
    Some(((control >> 12) as u8, control & 0x0004 != 0))
}

fn wifi_response(frame: &[u8]) -> Option<NativeWifiResponse> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let receiver = frame.get(4..10)?.try_into().ok()?;
    match frame_control & 0x00fc {
        0x00d4 => Some(NativeWifiResponse::Ack { receiver }),
        0x00c4 => Some(NativeWifiResponse::Cts { receiver }),
        0x0094 => Some(NativeWifiResponse::BlockAck {
            receiver,
            tid: block_ack_control(frame)?.0,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_frame(receiver: [u8; 6], transmitter: [u8; 6]) -> Vec<u8> {
        let mut frame = vec![0x08, 0x00, 0, 0];
        frame.extend_from_slice(&receiver);
        frame.extend_from_slice(&transmitter);
        frame.extend_from_slice(&[0; 8]);
        frame
    }

    #[test]
    fn unicast_data_requires_an_ack_to_the_transmitter() {
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let pending = PendingNativeWifiTransmission::new(
            2,
            &data_frame([0x02, 6, 7, 8, 9, 10], transmitter),
            None,
            SimTime::from_ticks(100),
            40,
        )
        .unwrap();
        assert_eq!(
            pending.expected_response,
            Some(NativeWifiResponse::Ack {
                receiver: transmitter
            })
        );
        assert_eq!(pending.deadline, SimTime::from_ticks(356));

        let mut ack = vec![0xd4, 0x00, 0, 0];
        ack.extend_from_slice(&transmitter);
        assert!(pending.accepts_response(&ack, SimTime::from_ticks(120)));
        assert!(!pending.accepts_response(&ack, SimTime::from_ticks(99)));
    }

    #[test]
    fn group_and_qos_no_ack_frames_complete_after_airtime() {
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let group = PendingNativeWifiTransmission::new(
            0,
            &data_frame([0xff; 6], transmitter),
            None,
            SimTime::from_ticks(100),
            40,
        )
        .unwrap();
        assert_eq!(group.expected_response, None);
        assert_eq!(group.deadline, group.end);

        let mut qos = data_frame([0x02, 6, 7, 8, 9, 10], transmitter);
        qos[0] = 0x88;
        qos.extend_from_slice(&[0; 2]);
        qos[24] = 1 << 5;
        let no_ack =
            PendingNativeWifiTransmission::new(0, &qos, None, SimTime::from_ticks(100), 40)
                .unwrap();
        assert_eq!(no_ack.expected_response, None);
    }

    #[test]
    fn rts_and_qos_block_ack_open_their_native_response_windows() {
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut rts = vec![0xb4, 0, 0, 0];
        rts.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
        rts.extend_from_slice(&transmitter);
        let pending =
            PendingNativeWifiTransmission::new(0, &rts, None, SimTime::from_ticks(100), 300)
                .unwrap();
        assert_eq!(
            pending.expected_response,
            Some(NativeWifiResponse::Cts {
                receiver: transmitter
            })
        );
        let cts = native_wifi_immediate_response(&rts).unwrap();
        assert_eq!(cts, [0xc4, 0, 0, 0, 2, 1, 2, 3, 4, 5]);
        assert!(pending.accepts_response(&cts, SimTime::from_ticks(110)));

        let mut qos = data_frame([0x02, 6, 7, 8, 9, 10], transmitter);
        qos[0] = 0x88;
        qos.extend_from_slice(&[0; 2]);
        qos[24] = (3 << 5) | 7;
        let pending =
            PendingNativeWifiTransmission::new(0, &qos, None, SimTime::from_ticks(200), 300)
                .unwrap();
        assert_eq!(
            pending.expected_response,
            Some(NativeWifiResponse::BlockAck {
                receiver: transmitter,
                tid: 7
            })
        );
        let request = NativeWifiBlockAckRequest {
            receiver: [0x02, 6, 7, 8, 9, 10],
            transmitter,
            tid: 6,
            starting_sequence: 0,
        };
        let wrong_tid = native_wifi_block_ack_response(request, 1);
        assert!(!pending.accepts_response(&wrong_tid, SimTime::from_ticks(210)));
    }

    #[test]
    fn descriptor_rts_runs_cts_then_payload_ack_as_two_hardware_phases() {
        let receiver = [0x02, 6, 7, 8, 9, 10];
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut payload = data_frame(receiver, transmitter);
        payload[2..4].copy_from_slice(&0x1234_u16.to_le_bytes());
        let rts = native_wifi_rts_protection(&payload).unwrap();
        assert_eq!(&rts[..4], &[0xb4, 0, 0x34, 0x12]);
        assert_eq!(&rts[4..10], &receiver);
        assert_eq!(&rts[10..16], &transmitter);

        let mut pending = PendingNativeWifiTransmission::new(
            3,
            &rts,
            Some(payload.clone()),
            SimTime::from_ticks(100),
            300,
        )
        .unwrap();
        assert!(pending.awaiting_protection_cts());
        assert_eq!(
            pending.deadline_outcome(),
            remu_devices::EspWifiTxOutcome::CtsTimeout
        );
        let cts = control_response(0xc4, transmitter);
        assert!(pending.accepts_response(&cts, SimTime::from_ticks(110)));

        let transmitted = pending
            .begin_protected_payload(SimTime::from_ticks(500))
            .unwrap();
        assert_eq!(transmitted, vec![payload]);
        assert!(!pending.awaiting_protection_cts());
        assert_eq!(
            pending.expected_response,
            Some(NativeWifiResponse::Ack {
                receiver: transmitter,
            })
        );
        assert_eq!(pending.deadline, SimTime::from_ticks(800));
        assert_eq!(
            pending.deadline_outcome(),
            remu_devices::EspWifiTxOutcome::AckTimeout
        );
    }

    #[test]
    fn descriptor_rts_rejects_group_and_control_payloads() {
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        assert!(native_wifi_rts_protection(&data_frame([0xff; 6], transmitter)).is_none());
        let mut ack = vec![0xd4, 0, 0, 0];
        ack.extend_from_slice(&transmitter);
        assert!(native_wifi_rts_protection(&ack).is_none());
    }

    #[test]
    fn fragmented_mpdus_open_independent_hardware_ack_windows() {
        let receiver = [0x02, 6, 7, 8, 9, 10];
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut first = data_frame(receiver, transmitter);
        first[1] |= 0x04;
        first[22..24].copy_from_slice(&((0x123_u16 << 4) | 0).to_le_bytes());
        let mut final_fragment = data_frame(receiver, transmitter);
        final_fragment[22..24].copy_from_slice(&((0x123_u16 << 4) | 1).to_le_bytes());

        for (queue, frame) in [(0, first), (1, final_fragment)] {
            let pending = PendingNativeWifiTransmission::new(
                queue,
                &frame,
                None,
                SimTime::from_ticks(100),
                300,
            )
            .unwrap();
            assert_eq!(
                pending.expected_response,
                Some(NativeWifiResponse::Ack {
                    receiver: transmitter,
                })
            );
            assert!(pending.accepts_response(
                &control_response(0xd4, transmitter),
                SimTime::from_ticks(110),
            ));
        }
    }

    #[test]
    fn aggregate_block_ack_counts_bitmap_hits_across_sequence_wrap() {
        let receiver = [0x02, 6, 7, 8, 9, 10];
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let frames = [0x0ffe_u16, 0x0fff, 0x0000]
            .into_iter()
            .map(|sequence| {
                let mut frame = data_frame(receiver, transmitter);
                frame[0] = 0x88;
                frame[22..24].copy_from_slice(&(sequence << 4).to_le_bytes());
                frame.extend_from_slice(&[((3 << 5) | 5), 0]);
                frame
            })
            .collect::<Vec<_>>();
        let pending =
            PendingNativeWifiTransmission::new_aggregate(2, &frames, SimTime::from_ticks(100), 300)
                .unwrap();
        let response = native_wifi_block_ack_response(
            NativeWifiBlockAckRequest {
                receiver,
                transmitter,
                tid: 5,
                starting_sequence: 0x0ffe,
            },
            0b101,
        );
        let (successful, record) = pending
            .response_completion(&response, SimTime::from_ticks(110))
            .unwrap();
        assert_eq!(successful, 2);
        assert_eq!(
            record,
            Some(remu_devices::EspWifiTxBlockAck {
                status: 0,
                starting_sequence: 0x0ffe,
                bitmap: 0b101,
            })
        );
    }

    #[test]
    fn aggregate_normal_ack_policy_opens_an_implicit_block_ack_window() {
        let receiver = [0x02, 6, 7, 8, 9, 10];
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let frames = [0x120_u16, 0x121]
            .into_iter()
            .map(|sequence| {
                let mut frame = data_frame(receiver, transmitter);
                frame[0] = 0x88;
                frame[22..24].copy_from_slice(&(sequence << 4).to_le_bytes());
                // QoS ACK policy zero is the implicit BA request when these
                // MPDUs are transmitted as one A-MPDU.
                frame.extend_from_slice(&[5, 0]);
                frame
            })
            .collect::<Vec<_>>();
        let pending =
            PendingNativeWifiTransmission::new_aggregate(2, &frames, SimTime::from_ticks(100), 300)
                .unwrap();
        assert_eq!(
            pending.expected_response,
            Some(NativeWifiResponse::BlockAck {
                receiver: transmitter,
                tid: 5,
            })
        );
    }

    #[test]
    fn aggregate_rejects_mixed_tid_and_duplicate_sequence_state() {
        let receiver = [0x02, 6, 7, 8, 9, 10];
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut first = data_frame(receiver, transmitter);
        first[0] = 0x88;
        first.extend_from_slice(&[((3 << 5) | 5), 0]);
        let duplicate = first.clone();
        assert!(
            PendingNativeWifiTransmission::new_aggregate(
                0,
                &[first.clone(), duplicate],
                SimTime::from_ticks(100),
                300,
            )
            .unwrap_err()
            .contains("repeats sequence")
        );
        let mut mixed_tid = first.clone();
        mixed_tid[24] = (3 << 5) | 6;
        assert!(
            PendingNativeWifiTransmission::new_aggregate(
                0,
                &[first, mixed_tid],
                SimTime::from_ticks(100),
                300,
            )
            .unwrap_err()
            .contains("receiver, transmitter, or TID")
        );
    }

    #[test]
    fn compressed_bar_builds_a_matching_block_ack() {
        let local = [0x02, 6, 7, 8, 9, 10];
        let peer = [0x02, 1, 2, 3, 4, 5];
        let mut bar = vec![0x84, 0, 0, 0];
        bar.extend_from_slice(&local);
        bar.extend_from_slice(&peer);
        bar.extend_from_slice(&(0x0004_u16 | (3_u16 << 12)).to_le_bytes());
        bar.extend_from_slice(&(0x123_u16 << 4).to_le_bytes());
        let request = native_wifi_block_ack_request(&bar).unwrap();
        assert_eq!(request.tid, 3);
        assert_eq!(request.starting_sequence, 0x123);
        let response = native_wifi_block_ack_response(request, 0x55);
        assert_eq!(&response[..2], &[0x94, 0]);
        assert_eq!(&response[4..10], &peer);
        assert_eq!(&response[10..16], &local);
        assert_eq!(&response[20..28], &0x55_u64.to_le_bytes());
    }

    #[test]
    fn qos_mpdu_exposes_the_native_scoreboard_key() {
        let local = [0x02, 6, 7, 8, 9, 10];
        let peer = [0x02, 1, 2, 3, 4, 5];
        let mut qos = data_frame(local, peer);
        qos[0] = 0x88;
        qos[22..24].copy_from_slice(&(0x345_u16 << 4).to_le_bytes());
        qos.extend_from_slice(&[7, 0]);
        assert_eq!(
            native_wifi_qos_mpdu(&qos),
            Some(NativeWifiQosMpdu {
                receiver: local,
                transmitter: peer,
                tid: 7,
                sequence: 0x345,
            })
        );
    }
}
