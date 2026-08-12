use remu_core::{SimDuration, SimTime};

/// One hardware-owned native Wi-Fi transmit operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingNativeWifiTransmission {
    pub(crate) queue: u8,
    pub(crate) end: SimTime,
    pub(crate) deadline: SimTime,
    pub(crate) expected_response: Option<NativeWifiResponse>,
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

impl PendingNativeWifiTransmission {
    /// Builds the completion window described by the outgoing MAC header and
    /// the guest-programmed per-queue timeout register.
    pub(crate) fn new(
        queue: u8,
        frame: &[u8],
        end: SimTime,
        programmed_timeout: u16,
    ) -> Option<Self> {
        let expected_response = response_for_transmit(frame);
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
        })
    }

    pub(crate) fn accepts_response(self, frame: &[u8], received_at: SimTime) -> bool {
        self.end <= received_at
            && received_at <= self.deadline
            && self.expected_response.is_some_and(|expected| {
                wifi_response(frame).is_some_and(|actual| actual == expected)
            })
    }
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
            PendingNativeWifiTransmission::new(0, &qos, SimTime::from_ticks(100), 40).unwrap();
        assert_eq!(no_ack.expected_response, None);
    }

    #[test]
    fn rts_and_qos_block_ack_open_their_native_response_windows() {
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut rts = vec![0xb4, 0, 0, 0];
        rts.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
        rts.extend_from_slice(&transmitter);
        let pending =
            PendingNativeWifiTransmission::new(0, &rts, SimTime::from_ticks(100), 300).unwrap();
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
            PendingNativeWifiTransmission::new(0, &qos, SimTime::from_ticks(200), 300).unwrap();
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
