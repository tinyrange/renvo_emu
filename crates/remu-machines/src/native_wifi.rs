use remu_core::{SimDuration, SimTime};

/// One hardware-owned native Wi-Fi transmit operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingNativeWifiTransmission {
    pub(crate) queue: u8,
    pub(crate) end: SimTime,
    pub(crate) deadline: SimTime,
    pub(crate) ack_receiver: Option<[u8; 6]>,
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
        let ack_receiver = ack_receiver_for_transmit(frame);
        // Bare-metal probes may leave the timeout field at reset. Hardware
        // still has a finite ACK window, so retain a small deterministic
        // fallback instead of completing an ACK-required frame immediately.
        let timeout = u64::from(programmed_timeout).max(256);
        let deadline = if ack_receiver.is_some() {
            end.checked_add(SimDuration::from_ticks(timeout)).ok()?
        } else {
            end
        };
        Some(Self {
            queue,
            end,
            deadline,
            ack_receiver,
        })
    }

    pub(crate) fn accepts_ack(self, frame: &[u8], received_at: SimTime) -> bool {
        self.end <= received_at
            && received_at <= self.deadline
            && self.ack_receiver.is_some_and(|receiver| {
                wifi_ack_receiver(frame).is_some_and(|actual| actual == receiver)
            })
    }
}

/// Returns the transmitter address that a normal peer ACK must name.
///
/// Management and unicast data frames use address two as the transmitter.
/// Group frames and QoS frames with a non-normal ACK policy complete after
/// airtime without opening an ACK window. Control-response exchanges such as
/// RTS/CTS are a separate hardware state machine and are not classified here.
fn ack_receiver_for_transmit(frame: &[u8]) -> Option<[u8; 6]> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    let frame_type = (frame_control >> 2) & 0x3;
    if frame_type != 0 && frame_type != 2 {
        return None;
    }
    let receiver: [u8; 6] = frame.get(4..10)?.try_into().ok()?;
    if receiver[0] & 1 != 0 {
        return None;
    }
    let subtype = (frame_control >> 4) & 0xf;
    if frame_type == 2 && subtype & 0x8 != 0 {
        let has_address_four = frame_control & (3 << 8) == 3 << 8;
        let qos_offset = if has_address_four { 30 } else { 24 };
        let qos_control = *frame.get(qos_offset)?;
        if qos_control >> 5 & 0x3 != 0 {
            return None;
        }
    }
    frame.get(10..16)?.try_into().ok()
}

/// Parses the receiver address from an 802.11 ACK control frame.
fn wifi_ack_receiver(frame: &[u8]) -> Option<[u8; 6]> {
    let frame_control = u16::from_le_bytes(frame.get(..2)?.try_into().ok()?);
    ((frame_control & 0x00fc) == 0x00d4)
        .then(|| frame.get(4..10)?.try_into().ok())
        .flatten()
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
        assert_eq!(pending.ack_receiver, Some(transmitter));
        assert_eq!(pending.deadline, SimTime::from_ticks(356));

        let mut ack = vec![0xd4, 0x00, 0, 0];
        ack.extend_from_slice(&transmitter);
        assert!(pending.accepts_ack(&ack, SimTime::from_ticks(120)));
        assert!(!pending.accepts_ack(&ack, SimTime::from_ticks(99)));
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
        assert_eq!(group.ack_receiver, None);
        assert_eq!(group.deadline, group.end);

        let mut qos = data_frame([0x02, 6, 7, 8, 9, 10], transmitter);
        qos[0] = 0x88;
        qos.extend_from_slice(&[0; 2]);
        qos[24] = 1 << 5;
        let no_ack =
            PendingNativeWifiTransmission::new(0, &qos, SimTime::from_ticks(100), 40).unwrap();
        assert_eq!(no_ack.ack_receiver, None);
    }
}
