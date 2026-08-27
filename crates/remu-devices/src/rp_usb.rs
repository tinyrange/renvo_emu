/// USB 2.0 packet identifier understood by the functional RP USB link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpUsbPid {
    /// Host-to-device transaction token.
    Out,
    /// Device-to-host transaction token.
    In,
    /// Start-of-frame token.
    Sof,
    /// Control-transfer setup token.
    Setup,
    /// Even data-toggle packet.
    Data0,
    /// Odd data-toggle packet.
    Data1,
    /// Successful transaction handshake.
    Ack,
    /// Temporarily unavailable transaction handshake.
    Nak,
    /// Halted endpoint handshake.
    Stall,
}

impl RpUsbPid {
    const fn nibble(self) -> u8 {
        match self {
            Self::Out => 0x1,
            Self::In => 0x9,
            Self::Sof => 0x5,
            Self::Setup => 0xd,
            Self::Data0 => 0x3,
            Self::Data1 => 0xb,
            Self::Ack => 0x2,
            Self::Nak => 0xa,
            Self::Stall => 0xe,
        }
    }

    /// Returns the on-wire PID byte, including its complemented high nibble.
    pub const fn byte(self) -> u8 {
        self.nibble() | ((!self.nibble() & 0xf) << 4)
    }

    fn decode(byte: u8) -> Result<Self, RpUsbPacketError> {
        if byte >> 4 != (!byte & 0xf) {
            return Err(RpUsbPacketError::PidComplement);
        }
        match byte & 0xf {
            0x1 => Ok(Self::Out),
            0x9 => Ok(Self::In),
            0x5 => Ok(Self::Sof),
            0xd => Ok(Self::Setup),
            0x3 => Ok(Self::Data0),
            0xb => Ok(Self::Data1),
            0x2 => Ok(Self::Ack),
            0xa => Ok(Self::Nak),
            0xe => Ok(Self::Stall),
            other => Err(RpUsbPacketError::UnsupportedPid(other)),
        }
    }
}

/// A validated USB packet at the functional SIE/PHY boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RpUsbPacket {
    /// Addressed token packet.
    Token {
        /// OUT, IN, or SETUP PID.
        pid: RpUsbPid,
        /// Seven-bit device address.
        address: u8,
        /// Four-bit endpoint number.
        endpoint: u8,
    },
    /// Start-of-frame packet with an 11-bit frame number.
    Sof(u16),
    /// DATA0 or DATA1 packet.
    Data {
        /// Data-toggle PID.
        pid: RpUsbPid,
        /// Packet payload, excluding CRC16.
        payload: Vec<u8>,
    },
    /// ACK, NAK, or STALL handshake packet.
    Handshake(RpUsbPid),
}

/// Packet validation error produced by the functional USB codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpUsbPacketError {
    /// The packet length is not valid for its PID family.
    Length,
    /// The PID complement nibble is invalid.
    PidComplement,
    /// The PID is validly complemented but not modeled.
    UnsupportedPid(u8),
    /// A PID appeared in the wrong packet family.
    PidFamily,
    /// A token's CRC5 is invalid.
    Crc5,
    /// A data packet's CRC16 is invalid.
    Crc16,
}

impl std::fmt::Display for RpUsbPacketError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for RpUsbPacketError {}

fn usb_crc5(value: u16) -> u8 {
    let mut crc = 0x1f_u8;
    for bit in 0..11 {
        let mix = (crc ^ ((value >> bit) as u8)) & 1;
        crc >>= 1;
        if mix != 0 {
            crc ^= 0x14;
        }
    }
    !crc & 0x1f
}

fn usb_crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        let mut byte = *byte;
        for _ in 0..8 {
            let mix = (crc ^ u16::from(byte)) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0xa001;
            }
            byte >>= 1;
        }
    }
    !crc
}

impl RpUsbPacket {
    /// Encodes the packet bytes from PID through CRC, excluding SYNC and EOP.
    pub fn encode(&self) -> Result<Vec<u8>, RpUsbPacketError> {
        match self {
            Self::Token {
                pid,
                address,
                endpoint,
            } => {
                if !matches!(pid, RpUsbPid::Out | RpUsbPid::In | RpUsbPid::Setup)
                    || *address > 0x7f
                    || *endpoint > 0xf
                {
                    return Err(RpUsbPacketError::PidFamily);
                }
                let token = u16::from(*address) | (u16::from(*endpoint) << 7);
                Ok(vec![
                    pid.byte(),
                    token as u8,
                    ((token >> 8) as u8 & 7) | (usb_crc5(token) << 3),
                ])
            }
            Self::Sof(frame) => {
                if *frame > 0x7ff {
                    return Err(RpUsbPacketError::PidFamily);
                }
                Ok(vec![
                    RpUsbPid::Sof.byte(),
                    *frame as u8,
                    ((*frame >> 8) as u8 & 7) | (usb_crc5(*frame) << 3),
                ])
            }
            Self::Data { pid, payload } => {
                if !matches!(pid, RpUsbPid::Data0 | RpUsbPid::Data1) {
                    return Err(RpUsbPacketError::PidFamily);
                }
                let mut bytes = Vec::with_capacity(payload.len() + 3);
                bytes.push(pid.byte());
                bytes.extend_from_slice(payload);
                bytes.extend_from_slice(&usb_crc16(payload).to_le_bytes());
                Ok(bytes)
            }
            Self::Handshake(pid) => {
                if !matches!(pid, RpUsbPid::Ack | RpUsbPid::Nak | RpUsbPid::Stall) {
                    return Err(RpUsbPacketError::PidFamily);
                }
                Ok(vec![pid.byte()])
            }
        }
    }

    /// Decodes and validates a PID/token/data/handshake byte sequence.
    pub fn decode(bytes: &[u8]) -> Result<Self, RpUsbPacketError> {
        let Some(first) = bytes.first() else {
            return Err(RpUsbPacketError::Length);
        };
        let pid = RpUsbPid::decode(*first)?;
        match pid {
            RpUsbPid::Out | RpUsbPid::In | RpUsbPid::Setup | RpUsbPid::Sof => {
                if bytes.len() != 3 {
                    return Err(RpUsbPacketError::Length);
                }
                let token = u16::from(bytes[1]) | (u16::from(bytes[2] & 7) << 8);
                if usb_crc5(token) != bytes[2] >> 3 {
                    return Err(RpUsbPacketError::Crc5);
                }
                if pid == RpUsbPid::Sof {
                    Ok(Self::Sof(token))
                } else {
                    Ok(Self::Token {
                        pid,
                        address: token as u8 & 0x7f,
                        endpoint: (token >> 7) as u8 & 0xf,
                    })
                }
            }
            RpUsbPid::Data0 | RpUsbPid::Data1 => {
                if bytes.len() < 3 {
                    return Err(RpUsbPacketError::Length);
                }
                let payload_end = bytes.len() - 2;
                let crc = u16::from_le_bytes([bytes[payload_end], bytes[payload_end + 1]]);
                if usb_crc16(&bytes[1..payload_end]) != crc {
                    return Err(RpUsbPacketError::Crc16);
                }
                Ok(Self::Data {
                    pid,
                    payload: bytes[1..payload_end].to_vec(),
                })
            }
            RpUsbPid::Ack | RpUsbPid::Nak | RpUsbPid::Stall => {
                if bytes.len() != 1 {
                    return Err(RpUsbPacketError::Length);
                }
                Ok(Self::Handshake(pid))
            }
        }
    }
}

/// Resolved full-speed USB differential line state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpUsbLineState {
    /// Both D+ and D- low; used for reset and EOP.
    Se0,
    /// Full-speed idle (D+ high, D- low).
    J,
    /// Opposite differential state (D+ low, D- high).
    K,
    /// Invalid state with both lines high.
    Se1,
}

impl RpUsbLineState {
    pub(super) const fn pins(self) -> (bool, bool) {
        match self {
            Self::Se0 => (false, false),
            Self::J => (true, false),
            Self::K => (false, true),
            Self::Se1 => (true, true),
        }
    }

    pub(super) const fn status_bits(self) -> u32 {
        let (dp, dm) = self.pins();
        (dp as u32) | ((dm as u32) << 1)
    }
}

#[derive(Clone, Debug)]
pub(super) struct RpUsbLinkState {
    pub(super) line: RpUsbLineState,
    pub(super) frame: u16,
    pub(super) suspended: bool,
    pub(super) toggles: [[bool; 16]; 2],
    pub(super) trace: Vec<RpUsbPacket>,
}

impl RpUsbLinkState {
    pub(super) fn reset() -> Self {
        Self {
            line: RpUsbLineState::Se0,
            frame: 0,
            suspended: false,
            toggles: [[false; 16]; 2],
            trace: Vec::new(),
        }
    }

    pub(super) fn bus_reset(&mut self) {
        self.line = RpUsbLineState::J;
        self.frame = 0;
        self.suspended = false;
        self.toggles = [[false; 16]; 2];
        self.trace.clear();
    }

    pub(super) fn record(&mut self, packet: RpUsbPacket) {
        self.trace.push(packet);
        self.line = RpUsbLineState::J;
    }

    pub(super) fn acknowledge(&mut self, endpoint: u8, input: bool) {
        if endpoint < 16 {
            self.toggles[usize::from(input)][usize::from(endpoint)] ^= true;
        }
    }
}

impl super::Rp2040UsbHandle {
    /// Advances the full-speed frame counter and records a validated SOF packet.
    pub fn inject_sof(&self) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.link.frame = state.link.frame.wrapping_add(1) & 0x7ff;
        let frame = state.link.frame;
        state.link.record(RpUsbPacket::Sof(frame));
    }

    /// Enters the deterministic suspended bus state.
    pub fn inject_suspend(&self) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.link.suspended = true;
        state.link.line = RpUsbLineState::J;
        state.registers[super::Rp2040UsbRegister::SieStatus.index()] |=
            super::Rp2040UsbState::SIE_STATUS_SUSPENDED;
    }

    /// Delivers full-speed resume signalling and leaves suspend.
    pub fn inject_resume(&self) {
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        state.link.suspended = false;
        state.link.line = RpUsbLineState::K;
        state.registers[super::Rp2040UsbRegister::SieStatus.index()] &=
            !super::Rp2040UsbState::SIE_STATUS_SUSPENDED;
        state.registers[super::Rp2040UsbRegister::SieStatus.index()] |=
            super::Rp2040UsbState::SIE_STATUS_RESUME;
    }

    /// Records a packet-level endpoint transaction and applies toggle/status semantics.
    pub fn transact(
        &self,
        endpoint: u8,
        input: bool,
        setup: bool,
        payload: &[u8],
        handshake: RpUsbPid,
    ) -> Result<(), RpUsbPacketError> {
        if endpoint >= 16 || !matches!(handshake, RpUsbPid::Ack | RpUsbPid::Nak | RpUsbPid::Stall) {
            return Err(RpUsbPacketError::PidFamily);
        }
        let mut state = self.state.lock().expect("RP2040 USB lock poisoned");
        let token = if setup {
            RpUsbPid::Setup
        } else if input {
            RpUsbPid::In
        } else {
            RpUsbPid::Out
        };
        let data = if setup || !state.link.toggles[usize::from(input)][usize::from(endpoint)] {
            RpUsbPid::Data0
        } else {
            RpUsbPid::Data1
        };
        for packet in [
            RpUsbPacket::Token {
                pid: token,
                address: 0,
                endpoint,
            },
            RpUsbPacket::Data {
                pid: data,
                payload: payload.to_vec(),
            },
            RpUsbPacket::Handshake(handshake),
        ] {
            let encoded = packet.encode()?;
            RpUsbPacket::decode(&encoded)?;
            state.link.record(packet);
        }
        let status_index = super::Rp2040UsbRegister::SieStatus.index();
        match handshake {
            RpUsbPid::Ack => {
                state.registers[status_index] |= super::Rp2040UsbState::SIE_STATUS_ACK_REC
                    | super::Rp2040UsbState::SIE_STATUS_TRANS_COMPLETE;
                if setup {
                    state.link.toggles[0][0] = true;
                    state.link.toggles[1][0] = true;
                } else {
                    state.link.acknowledge(endpoint, input);
                }
            }
            RpUsbPid::Nak | RpUsbPid::Stall => {
                state.registers[status_index] |= if handshake == RpUsbPid::Nak {
                    super::Rp2040UsbState::SIE_STATUS_NAK_REC
                } else {
                    super::Rp2040UsbState::SIE_STATUS_STALL_REC
                };
                state.registers[super::Rp2040UsbRegister::EpStatusStallNak.index()] |=
                    1 << (u32::from(endpoint) * 2 + u32::from(!input));
            }
            _ => unreachable!("handshake was validated above"),
        }
        Ok(())
    }

    /// Reports a receive error through SIE status and interrupt state.
    pub fn inject_receive_error(&self, error: RpUsbPacketError) {
        let bit = match error {
            RpUsbPacketError::Crc5 | RpUsbPacketError::Crc16 => {
                super::Rp2040UsbState::SIE_STATUS_CRC_ERROR
            }
            RpUsbPacketError::PidComplement | RpUsbPacketError::UnsupportedPid(_) => {
                super::Rp2040UsbState::SIE_STATUS_DATA_SEQ_ERROR
            }
            RpUsbPacketError::Length | RpUsbPacketError::PidFamily => {
                super::Rp2040UsbState::SIE_STATUS_RX_OVERFLOW
            }
        };
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .registers[super::Rp2040UsbRegister::SieStatus.index()] |= bit;
    }

    /// Returns the current differential bus state.
    pub fn line_state(&self) -> RpUsbLineState {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .link
            .line
    }

    /// Returns the current data-toggle selection for an endpoint direction.
    pub fn data_toggle(&self, endpoint: u8, input: bool) -> Option<bool> {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .link
            .toggles
            .get(usize::from(input))?
            .get(usize::from(endpoint))
            .copied()
    }

    /// Returns the validated packet trace observed by the functional link.
    pub fn packet_trace(&self) -> Vec<RpUsbPacket> {
        self.state
            .lock()
            .expect("RP2040 USB lock poisoned")
            .link
            .trace
            .clone()
    }
}
