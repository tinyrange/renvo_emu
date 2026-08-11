use aes::Aes128;
use ccm::{
    Ccm,
    aead::{AeadInOut, KeyInit},
    consts::{U4, U8, U13, U16},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// IEEE 802.15.4 short address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShortAddress(pub u16);

/// IEEE 802.15.4 extended address in least-significant-byte-first wire order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExtendedAddress(pub [u8; 8]);

/// One enabled PAN/address filter slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanInterface {
    /// Personal-area-network identifier.
    pub pan_id: u16,
    /// Local short address.
    pub short_address: ShortAddress,
    /// Local extended address.
    pub extended_address: ExtendedAddress,
}

/// AES-CCM* key and nonce inputs for one secured operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityMaterial {
    /// AES-128 key.
    pub key: [u8; 16],
    /// Source extended address used by the nonce.
    pub source: ExtendedAddress,
    /// Monotonic frame counter used by the nonce.
    pub frame_counter: u32,
    /// IEEE security level from zero through seven.
    pub level: u8,
}

/// Hardware clear-channel-assessment policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ieee802154CcaMode {
    /// Detect an IEEE 802.15.4 carrier regardless of the ED threshold.
    Carrier,
    /// Compare received energy with the programmed ED threshold.
    Energy,
    /// Report busy when either carrier or energy detection is positive.
    CarrierOrEnergy,
    /// Report busy only when carrier and energy detection are both positive.
    CarrierAndEnergy,
}

/// Deterministic frame validation or security error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Ieee802154Error {
    /// Physical service data unit is outside the legal 3..=127 byte range.
    #[error("invalid IEEE 802.15.4 frame length {0}")]
    InvalidLength(usize),
    /// Header addressing fields are truncated or reserved.
    #[error("malformed IEEE 802.15.4 MAC header")]
    MalformedHeader,
    /// Supplied frame check sequence is invalid.
    #[error("IEEE 802.15.4 FCS mismatch")]
    InvalidFcs,
    /// Security level is outside the standardized zero-through-seven range.
    #[error("reserved IEEE 802.15.4 security level {0}")]
    InvalidSecurityLevel(u8),
    /// AES-CCM* authentication failed.
    #[error("IEEE 802.15.4 security authentication failed")]
    AuthenticationFailed,
    /// Secured frame does not contain its complete auxiliary header and MIC.
    #[error("malformed IEEE 802.15.4 security fields")]
    MalformedSecurity,
    /// Transmit security was requested for a frame without its FCF security bit.
    #[error("IEEE 802.15.4 frame-control security bit is not set")]
    SecurityNotEnabled,
    /// The auxiliary security header suppresses the frame counter required by C6.
    #[error("IEEE 802.15.4 frame counter is suppressed")]
    SecurityCounterSuppressed,
    /// The hardware payload offset does not follow the auxiliary security header.
    #[error("invalid IEEE 802.15.4 security payload offset {0}")]
    InvalidSecurityOffset(usize),
}

/// Receive disposition after FCS, address, and security handling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Ieee802154RxOutcome {
    /// Frame was accepted and no automatic response is needed.
    Accepted {
        /// MAC frame without the two-byte FCS.
        frame: Vec<u8>,
        /// Matching PAN interface, absent in promiscuous mode.
        interface: Option<u8>,
    },
    /// Frame was accepted and generated a standard immediate ACK.
    AcceptedWithAck {
        /// MAC frame without the two-byte FCS.
        frame: Vec<u8>,
        /// Matching PAN interface, absent in promiscuous mode.
        interface: Option<u8>,
        /// Complete ACK including its calculated FCS.
        ack: Vec<u8>,
    },
    /// Frame was well formed but rejected by PAN/address filtering.
    Filtered,
}

/// Append-only IEEE 802.15.4 MAC evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Ieee802154Event {
    /// Receive frame passed all enabled checks.
    RxAccepted {
        /// PSDU length including FCS.
        length: usize,
        /// Matching PAN slot.
        interface: Option<u8>,
    },
    /// Receive frame was rejected by destination filtering.
    RxFiltered,
    /// Receive frame had a bad FCS.
    RxBadFcs,
    /// An automatic ACK was generated.
    AckGenerated {
        /// Acknowledged data sequence number.
        sequence: u8,
        /// Frame-pending bit copied from MAC policy.
        frame_pending: bool,
    },
    /// AES-CCM* protection was applied.
    SecurityProtected {
        /// IEEE security level.
        level: u8,
        /// Generated message-integrity-code length.
        mic_length: usize,
    },
    /// AES-CCM* verification succeeded.
    SecurityVerified {
        /// IEEE security level.
        level: u8,
    },
    /// Clear-channel assessment sampled the medium.
    Cca {
        /// Whether received energy exceeded the programmed threshold.
        busy: bool,
        /// Sampled received energy in dBm.
        energy_dbm: i16,
        /// Whether a compatible IEEE 802.15.4 carrier was present.
        carrier: bool,
        /// Programmed hardware detection mode.
        mode: Ieee802154CcaMode,
    },
}

/// Functional IEEE 802.15.4 MAC filter, ACK, CCA, and AES-CCM* engine.
#[derive(Clone, Debug, Default)]
pub struct Ieee802154Mac {
    interfaces: [Option<PanInterface>; 4],
    promiscuous: bool,
    auto_ack: bool,
    frame_pending: bool,
    cca_threshold_dbm: i16,
    events: Vec<Ieee802154Event>,
}

impl Ieee802154Mac {
    /// Creates a reset MAC with an empty address table.
    pub fn new() -> Self {
        Self {
            interfaces: [None; 4],
            promiscuous: false,
            auto_ack: false,
            frame_pending: false,
            cca_threshold_dbm: -75,
            events: Vec::new(),
        }
    }

    /// Replaces one of four hardware PAN interfaces.
    pub fn set_interface(
        &mut self,
        index: u8,
        interface: Option<PanInterface>,
    ) -> Result<(), Ieee802154Error> {
        let slot = self
            .interfaces
            .get_mut(usize::from(index))
            .ok_or(Ieee802154Error::MalformedHeader)?;
        *slot = interface;
        Ok(())
    }

    /// Enables raw receive mode without PAN/address rejection.
    pub fn set_promiscuous(&mut self, enabled: bool) {
        self.promiscuous = enabled;
    }

    /// Enables standard immediate ACK generation for requested unicast frames.
    pub fn set_auto_ack(&mut self, enabled: bool) {
        self.auto_ack = enabled;
    }

    /// Selects the frame-pending bit used in automatic ACKs.
    pub fn set_frame_pending(&mut self, pending: bool) {
        self.frame_pending = pending;
    }

    /// Sets the energy threshold used by deterministic clear-channel assessment.
    pub fn set_cca_threshold_dbm(&mut self, threshold: i16) {
        self.cca_threshold_dbm = threshold;
    }

    /// Samples energy and returns true when the channel is busy.
    pub fn clear_channel_assessment(&mut self, energy_dbm: i16) -> bool {
        self.clear_channel_assessment_with_mode(energy_dbm, false, Ieee802154CcaMode::Energy)
    }

    /// Samples energy and carrier state using the selected hardware CCA mode.
    pub fn clear_channel_assessment_with_mode(
        &mut self,
        energy_dbm: i16,
        carrier: bool,
        mode: Ieee802154CcaMode,
    ) -> bool {
        let energy = energy_dbm >= self.cca_threshold_dbm;
        let busy = match mode {
            Ieee802154CcaMode::Carrier => carrier,
            Ieee802154CcaMode::Energy => energy,
            Ieee802154CcaMode::CarrierOrEnergy => carrier || energy,
            Ieee802154CcaMode::CarrierAndEnergy => carrier && energy,
        };
        self.events.push(Ieee802154Event::Cca {
            busy,
            energy_dbm,
            carrier,
            mode,
        });
        busy
    }

    /// Appends the standardized two-byte frame check sequence.
    #[must_use]
    pub fn with_fcs(mut mac_frame: Vec<u8>) -> Vec<u8> {
        mac_frame.extend_from_slice(&ieee802154_fcs(&mac_frame).to_le_bytes());
        mac_frame
    }

    /// Verifies the final two bytes as an IEEE 802.15.4 frame check sequence.
    pub fn has_valid_fcs(psdu: &[u8]) -> bool {
        let Some(frame_length) = psdu.len().checked_sub(2) else {
            return false;
        };
        ieee802154_fcs(&psdu[..frame_length])
            == u16::from_le_bytes([psdu[frame_length], psdu[frame_length + 1]])
    }

    /// Validates FCS/addressing and optionally generates an immediate ACK.
    pub fn receive(&mut self, psdu: &[u8]) -> Result<Ieee802154RxOutcome, Ieee802154Error> {
        if !(5..=127).contains(&psdu.len()) {
            return Err(Ieee802154Error::InvalidLength(psdu.len()));
        }
        let frame_length = psdu.len() - 2;
        let expected = u16::from_le_bytes([psdu[frame_length], psdu[frame_length + 1]]);
        if ieee802154_fcs(&psdu[..frame_length]) != expected {
            self.events.push(Ieee802154Event::RxBadFcs);
            return Err(Ieee802154Error::InvalidFcs);
        }
        let header = ParsedHeader::parse(&psdu[..frame_length])?;
        let interface = if self.promiscuous {
            None
        } else if let Some(index) = self.matching_interface(&header) {
            Some(index)
        } else {
            self.events.push(Ieee802154Event::RxFiltered);
            return Ok(Ieee802154RxOutcome::Filtered);
        };
        self.events.push(Ieee802154Event::RxAccepted {
            length: psdu.len(),
            interface,
        });
        let frame = psdu[..frame_length].to_vec();
        if self.auto_ack && header.ack_request && !header.destination_is_broadcast() {
            let sequence = header.sequence.ok_or(Ieee802154Error::MalformedHeader)?;
            let ack = make_ack(sequence, self.frame_pending);
            self.events.push(Ieee802154Event::AckGenerated {
                sequence,
                frame_pending: self.frame_pending,
            });
            Ok(Ieee802154RxOutcome::AcceptedWithAck {
                frame,
                interface,
                ack,
            })
        } else {
            Ok(Ieee802154RxOutcome::Accepted { frame, interface })
        }
    }

    /// Protects a MAC payload using IEEE 802.15.4 AES-CCM* security levels.
    ///
    /// `authenticated_header` contains the complete MAC header and auxiliary
    /// security header; `payload` is modified in place and the returned value
    /// is the MIC to append.
    pub fn protect(
        &mut self,
        authenticated_header: &[u8],
        payload: &mut Vec<u8>,
        material: SecurityMaterial,
    ) -> Result<Vec<u8>, Ieee802154Error> {
        validate_security_level(material.level)?;
        if material.level == 0 {
            return Ok(Vec::new());
        }
        let nonce = security_nonce(material);
        let encrypt = material.level >= 4;
        let mic_length = security_mic_length(material.level);
        let mut transformed = payload.clone();
        let tag = match mic_length {
            0 => ccm_encrypt::<U4>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
            )?[..0]
                .to_vec(),
            4 => ccm_encrypt::<U4>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
            )?,
            8 => ccm_encrypt::<U8>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
            )?,
            16 => ccm_encrypt::<U16>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
            )?,
            _ => unreachable!("security level maps to a supported MIC size"),
        };
        if encrypt {
            *payload = transformed;
        }
        self.events.push(Ieee802154Event::SecurityProtected {
            level: material.level,
            mic_length,
        });
        Ok(tag)
    }

    /// Applies the C6 transmit-security contract to a MAC frame without FCS.
    ///
    /// `payload_offset` is measured from the first MAC byte. The ESP-IDF frame
    /// parser removes the preceding PHY length byte before programming the C6
    /// register. The security level and frame counter are taken from the
    /// frame's auxiliary security header, while the key and nonce source are
    /// supplied by the hardware security registers. The caller-provided frame
    /// includes the MIC reservation selected by the security level; hardware
    /// overwrites that reservation instead of increasing the PHY length.
    pub fn protect_transmit_frame(
        &mut self,
        frame: &[u8],
        payload_offset: usize,
        key: [u8; 16],
        source: ExtendedAddress,
    ) -> Result<Vec<u8>, Ieee802154Error> {
        let fcf = frame
            .get(..2)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or(Ieee802154Error::MalformedHeader)?;
        if fcf & (1 << 3) == 0 {
            return Err(Ieee802154Error::SecurityNotEnabled);
        }
        let security_offset = auxiliary_security_header_offset(frame)?;
        let security_control = *frame
            .get(security_offset)
            .ok_or(Ieee802154Error::MalformedSecurity)?;
        if security_control & (1 << 5) != 0 {
            return Err(Ieee802154Error::SecurityCounterSuppressed);
        }
        let counter_end = security_offset
            .checked_add(5)
            .ok_or(Ieee802154Error::MalformedSecurity)?;
        let counter: [u8; 4] = frame
            .get(security_offset + 1..counter_end)
            .ok_or(Ieee802154Error::MalformedSecurity)?
            .try_into()
            .expect("checked four-byte frame counter");
        if payload_offset < counter_end || payload_offset > frame.len() {
            return Err(Ieee802154Error::InvalidSecurityOffset(payload_offset));
        }

        let level = security_control & 7;
        if level == 0 {
            return Err(Ieee802154Error::InvalidSecurityLevel(level));
        }
        let mic_length = security_mic_length(level);
        let payload_end = frame
            .len()
            .checked_sub(mic_length)
            .filter(|end| *end >= payload_offset)
            .ok_or(Ieee802154Error::InvalidLength(frame.len()))?;
        let material = SecurityMaterial {
            key,
            source,
            frame_counter: u32::from_le_bytes(counter),
            level,
        };
        let mut protected = frame[..payload_offset].to_vec();
        let mut payload = frame[payload_offset..payload_end].to_vec();
        let mic = self.protect(&protected, &mut payload, material)?;
        let final_length = protected
            .len()
            .saturating_add(payload.len())
            .saturating_add(mic.len());
        if final_length > 125 || final_length != frame.len() {
            return Err(Ieee802154Error::InvalidLength(final_length));
        }
        protected.extend_from_slice(&payload);
        protected.extend_from_slice(&mic);
        Ok(protected)
    }

    /// Verifies and decrypts an AES-CCM* protected MAC payload.
    pub fn unprotect(
        &mut self,
        authenticated_header: &[u8],
        payload: &mut Vec<u8>,
        mic: &[u8],
        material: SecurityMaterial,
    ) -> Result<(), Ieee802154Error> {
        validate_security_level(material.level)?;
        let mic_length = security_mic_length(material.level);
        if mic.len() != mic_length {
            return Err(Ieee802154Error::MalformedSecurity);
        }
        if material.level == 0 {
            return Ok(());
        }
        let nonce = security_nonce(material);
        let encrypted = material.level >= 4;
        let mut transformed = payload.clone();
        let valid = match (encrypted, mic_length) {
            (true, 0) => {
                let _ = ccm_encrypt::<U4>(
                    &material.key,
                    &nonce,
                    authenticated_header,
                    &mut transformed,
                )?;
                true
            }
            (false, 4) => ccm_authenticate::<U4>(
                &material.key,
                &nonce,
                authenticated_header,
                &transformed,
                mic,
            )?,
            (false, 8) => ccm_authenticate::<U8>(
                &material.key,
                &nonce,
                authenticated_header,
                &transformed,
                mic,
            )?,
            (false, 16) => ccm_authenticate::<U16>(
                &material.key,
                &nonce,
                authenticated_header,
                &transformed,
                mic,
            )?,
            (true, 4) => ccm_decrypt::<U4>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
                mic,
            ),
            (true, 8) => ccm_decrypt::<U8>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
                mic,
            ),
            (true, 16) => ccm_decrypt::<U16>(
                &material.key,
                &nonce,
                authenticated_header,
                &mut transformed,
                mic,
            ),
            _ => unreachable!("security level maps to a supported MIC size"),
        };
        if !valid {
            return Err(Ieee802154Error::AuthenticationFailed);
        }
        if encrypted {
            *payload = transformed;
        }
        self.events.push(Ieee802154Event::SecurityVerified {
            level: material.level,
        });
        Ok(())
    }

    /// Append-only MAC behavior evidence.
    pub fn events(&self) -> &[Ieee802154Event] {
        &self.events
    }

    /// Resets volatile configuration and zeroizes address/filter state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn matching_interface(&self, header: &ParsedHeader) -> Option<u8> {
        if header.destination.is_none() {
            return Some(0);
        }
        self.interfaces
            .iter()
            .enumerate()
            .filter_map(|(index, interface)| interface.map(|interface| (index, interface)))
            .find(|(_, interface)| header.matches(*interface))
            .map(|(index, _)| u8::try_from(index).expect("four PAN slots fit in u8"))
    }
}

#[derive(Clone, Copy, Debug)]
enum MacAddress {
    Short(u16),
    Extended([u8; 8]),
}

#[derive(Clone, Copy, Debug)]
struct ParsedHeader {
    sequence: Option<u8>,
    ack_request: bool,
    destination_pan: Option<u16>,
    destination: Option<MacAddress>,
}

impl ParsedHeader {
    fn parse(frame: &[u8]) -> Result<Self, Ieee802154Error> {
        if frame.len() < 2 {
            return Err(Ieee802154Error::MalformedHeader);
        }
        let fcf = u16::from_le_bytes([frame[0], frame[1]]);
        let frame_version = ((fcf >> 12) & 3) as u8;
        let sequence_suppressed = frame_version == 2 && fcf & (1 << 8) != 0;
        let destination_mode = ((fcf >> 10) & 3) as u8;
        if destination_mode == 1 {
            return Err(Ieee802154Error::MalformedHeader);
        }
        let mut cursor = 2;
        let sequence = if sequence_suppressed {
            None
        } else {
            let value = *frame.get(cursor).ok_or(Ieee802154Error::MalformedHeader)?;
            cursor += 1;
            Some(value)
        };
        let (destination_pan, destination) = match destination_mode {
            0 => (None, None),
            2 => {
                let pan = read_u16(frame, &mut cursor)?;
                let address = read_u16(frame, &mut cursor)?;
                (Some(pan), Some(MacAddress::Short(address)))
            }
            3 => {
                let pan = read_u16(frame, &mut cursor)?;
                let address = read_array::<8>(frame, &mut cursor)?;
                (Some(pan), Some(MacAddress::Extended(address)))
            }
            _ => return Err(Ieee802154Error::MalformedHeader),
        };
        Ok(Self {
            sequence,
            ack_request: fcf & (1 << 5) != 0,
            destination_pan,
            destination,
        })
    }

    fn matches(self, interface: PanInterface) -> bool {
        let pan_matches = self
            .destination_pan
            .is_none_or(|pan| pan == interface.pan_id || pan == 0xffff);
        let address_matches = match self.destination {
            None => true,
            Some(MacAddress::Short(address)) => {
                address == interface.short_address.0 || address == 0xffff
            }
            Some(MacAddress::Extended(address)) => address == interface.extended_address.0,
        };
        pan_matches && address_matches
    }

    fn destination_is_broadcast(self) -> bool {
        matches!(self.destination, Some(MacAddress::Short(0xffff)))
    }
}

fn auxiliary_security_header_offset(frame: &[u8]) -> Result<usize, Ieee802154Error> {
    if frame.len() < 2 {
        return Err(Ieee802154Error::MalformedHeader);
    }
    let fcf = u16::from_le_bytes([frame[0], frame[1]]);
    if fcf & (1 << 3) == 0 {
        return Err(Ieee802154Error::MalformedSecurity);
    }
    let frame_version = ((fcf >> 12) & 3) as u8;
    if frame_version == 3 {
        return Err(Ieee802154Error::MalformedHeader);
    }
    let destination_mode = ((fcf >> 10) & 3) as u8;
    let source_mode = ((fcf >> 14) & 3) as u8;
    if destination_mode == 1 || source_mode == 1 {
        return Err(Ieee802154Error::MalformedHeader);
    }
    let pan_compression = fcf & (1 << 6) != 0;
    let sequence_present = frame_version != 2 || fcf & (1 << 8) == 0;
    let mut cursor = 2 + usize::from(sequence_present);

    let destination_pan_present = if frame_version == 2 {
        if destination_mode != 0 {
            !((source_mode == 0 && pan_compression)
                || (destination_mode == 3 && source_mode == 3 && pan_compression))
        } else {
            source_mode == 0 && pan_compression
        }
    } else {
        destination_mode != 0
    };
    let source_pan_present = if frame_version == 2 {
        source_mode != 0 && !pan_compression && !(destination_mode == 3 && source_mode == 3)
    } else {
        source_mode != 0 && !pan_compression
    };
    cursor = cursor
        .checked_add(usize::from(destination_pan_present) * 2)
        .and_then(|value| value.checked_add(address_length(destination_mode)))
        .and_then(|value| value.checked_add(usize::from(source_pan_present) * 2))
        .and_then(|value| value.checked_add(address_length(source_mode)))
        .ok_or(Ieee802154Error::MalformedHeader)?;
    if cursor >= frame.len() {
        return Err(Ieee802154Error::MalformedSecurity);
    }
    Ok(cursor)
}

fn address_length(mode: u8) -> usize {
    match mode {
        2 => 2,
        3 => 8,
        _ => 0,
    }
}

fn read_u16(frame: &[u8], cursor: &mut usize) -> Result<u16, Ieee802154Error> {
    let bytes = read_array::<2>(frame, cursor)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_array<const N: usize>(
    frame: &[u8],
    cursor: &mut usize,
) -> Result<[u8; N], Ieee802154Error> {
    let end = cursor
        .checked_add(N)
        .ok_or(Ieee802154Error::MalformedHeader)?;
    let value = frame
        .get(*cursor..end)
        .ok_or(Ieee802154Error::MalformedHeader)?
        .try_into()
        .expect("checked fixed-size field");
    *cursor = end;
    Ok(value)
}

fn make_ack(sequence: u8, pending: bool) -> Vec<u8> {
    let mut ack = vec![0x02 | (u8::from(pending) << 4), 0x00, sequence];
    ack.extend_from_slice(&ieee802154_fcs(&ack).to_le_bytes());
    ack
}

fn ieee802154_fcs(frame: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in frame {
        let mut value = u16::from(*byte);
        for _ in 0..8 {
            let mix = (crc ^ value) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8408;
            }
            value >>= 1;
        }
    }
    crc
}

fn security_nonce(material: SecurityMaterial) -> [u8; 13] {
    let mut nonce = [0_u8; 13];
    nonce[..8].copy_from_slice(&material.source.0);
    nonce[8..12].copy_from_slice(&material.frame_counter.to_le_bytes());
    nonce[12] = material.level;
    nonce
}

fn validate_security_level(level: u8) -> Result<(), Ieee802154Error> {
    if level <= 7 {
        Ok(())
    } else {
        Err(Ieee802154Error::InvalidSecurityLevel(level))
    }
}

fn security_mic_length(level: u8) -> usize {
    match level & 3 {
        0 => 0,
        1 => 4,
        2 => 8,
        3 => 16,
        _ => unreachable!(),
    }
}

fn ccm_encrypt<TagSize>(
    key: &[u8; 16],
    nonce: &[u8; 13],
    aad: &[u8],
    payload: &mut [u8],
) -> Result<Vec<u8>, Ieee802154Error>
where
    TagSize: ccm::TagSize + ccm::aead::array::ArraySize,
{
    let cipher = Ccm::<Aes128, TagSize, U13>::new(key.into());
    cipher
        .encrypt_inout_detached(nonce.into(), aad, payload.into())
        .map(|tag| tag.to_vec())
        .map_err(|_| Ieee802154Error::AuthenticationFailed)
}

fn ccm_decrypt<TagSize>(
    key: &[u8; 16],
    nonce: &[u8; 13],
    aad: &[u8],
    payload: &mut [u8],
    mic: &[u8],
) -> bool
where
    TagSize: ccm::TagSize + ccm::aead::array::ArraySize,
{
    let cipher = Ccm::<Aes128, TagSize, U13>::new(key.into());
    let Ok(tag) = mic.try_into() else {
        return false;
    };
    cipher
        .decrypt_inout_detached(nonce.into(), aad, payload.into(), tag)
        .is_ok()
}

fn ccm_authenticate<TagSize>(
    key: &[u8; 16],
    nonce: &[u8; 13],
    aad: &[u8],
    payload: &[u8],
    mic: &[u8],
) -> Result<bool, Ieee802154Error>
where
    TagSize: ccm::TagSize + ccm::aead::array::ArraySize,
{
    let mut scratch = payload.to_vec();
    ccm_encrypt::<TagSize>(key, nonce, aad, &mut scratch).map(|expected| expected == mic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> PanInterface {
        PanInterface {
            pan_id: 0x1234,
            short_address: ShortAddress(0x5678),
            extended_address: ExtendedAddress([1, 2, 3, 4, 5, 6, 7, 8]),
        }
    }

    fn data_frame(destination_pan: u16, destination: u16, ack: bool) -> Vec<u8> {
        let fcf = 0x0801_u16 | (u16::from(ack) << 5);
        let mut frame = Vec::from(fcf.to_le_bytes());
        frame.push(0x2a);
        frame.extend_from_slice(&destination_pan.to_le_bytes());
        frame.extend_from_slice(&destination.to_le_bytes());
        frame.extend_from_slice(b"payload");
        frame.extend_from_slice(&ieee802154_fcs(&frame).to_le_bytes());
        frame
    }

    #[test]
    fn pan_filter_and_promiscuous_paths_are_explicit() {
        let mut mac = Ieee802154Mac::new();
        mac.set_interface(2, Some(interface())).unwrap();
        assert!(matches!(
            mac.receive(&data_frame(0x1234, 0x5678, false)).unwrap(),
            Ieee802154RxOutcome::Accepted {
                interface: Some(2),
                ..
            }
        ));
        assert_eq!(
            mac.receive(&data_frame(0xabcd, 0x5678, false)).unwrap(),
            Ieee802154RxOutcome::Filtered
        );
        mac.set_promiscuous(true);
        assert!(matches!(
            mac.receive(&data_frame(0xabcd, 0x5678, false)).unwrap(),
            Ieee802154RxOutcome::Accepted {
                interface: None,
                ..
            }
        ));
    }

    #[test]
    fn auto_ack_has_sequence_pending_and_valid_fcs() {
        let mut mac = Ieee802154Mac::new();
        mac.set_interface(0, Some(interface())).unwrap();
        mac.set_auto_ack(true);
        mac.set_frame_pending(true);
        let Ieee802154RxOutcome::AcceptedWithAck { ack, .. } =
            mac.receive(&data_frame(0x1234, 0x5678, true)).unwrap()
        else {
            panic!("ACK requested for matching unicast frame");
        };
        assert_eq!(&ack[..3], &[0x12, 0, 0x2a]);
        assert_eq!(
            ieee802154_fcs(&ack[..3]),
            u16::from_le_bytes([ack[3], ack[4]])
        );
    }

    #[test]
    fn fcs_and_cca_failures_are_observable() {
        let mut mac = Ieee802154Mac::new();
        let mut frame = data_frame(0xffff, 0xffff, false);
        frame[3] ^= 1;
        assert_eq!(mac.receive(&frame), Err(Ieee802154Error::InvalidFcs));
        mac.set_cca_threshold_dbm(-80);
        assert!(!mac.clear_channel_assessment(-81));
        assert!(mac.clear_channel_assessment(-80));
    }

    #[test]
    fn cca_modes_distinguish_carrier_from_energy() {
        let mut mac = Ieee802154Mac::new();
        mac.set_cca_threshold_dbm(-75);
        assert!(mac.clear_channel_assessment_with_mode(-90, true, Ieee802154CcaMode::Carrier));
        assert!(!mac.clear_channel_assessment_with_mode(-90, true, Ieee802154CcaMode::Energy));
        assert!(mac.clear_channel_assessment_with_mode(
            -70,
            false,
            Ieee802154CcaMode::CarrierOrEnergy
        ));
        assert!(!mac.clear_channel_assessment_with_mode(
            -70,
            false,
            Ieee802154CcaMode::CarrierAndEnergy
        ));
    }

    #[test]
    fn aes_ccm_star_round_trips_and_rejects_wrong_mic() {
        let material = SecurityMaterial {
            key: [0x11; 16],
            source: ExtendedAddress([1, 2, 3, 4, 5, 6, 7, 8]),
            frame_counter: 7,
            level: 5,
        };
        let header = [0x49, 0x88, 1, 2, 3, 4, 5];
        let original = b"secured payload".to_vec();
        let mut protected = original.clone();
        let mut mac = Ieee802154Mac::new();
        let mic = mac.protect(&header, &mut protected, material).unwrap();
        assert_ne!(protected, original);
        assert_eq!(mic.len(), 4);
        mac.unprotect(&header, &mut protected, &mic, material)
            .unwrap();
        assert_eq!(protected, original);

        let mut protected = original.clone();
        let mut mic = mac.protect(&header, &mut protected, material).unwrap();
        mic[0] ^= 1;
        assert_eq!(
            mac.unprotect(&header, &mut protected, &mic, material),
            Err(Ieee802154Error::AuthenticationFailed)
        );
    }

    #[test]
    fn authentication_only_does_not_encrypt_payload() {
        let material = SecurityMaterial {
            key: [0x22; 16],
            source: ExtendedAddress([8, 7, 6, 5, 4, 3, 2, 1]),
            frame_counter: 9,
            level: 2,
        };
        let original = b"authenticate only".to_vec();
        let mut payload = original.clone();
        let mut mac = Ieee802154Mac::new();
        let mic = mac.protect(b"header", &mut payload, material).unwrap();
        assert_eq!(payload, original);
        assert_eq!(mic.len(), 8);
        mac.unprotect(b"header", &mut payload, &mic, material)
            .unwrap();
        let mut invalid_mic = mic;
        invalid_mic[2] ^= 1;
        assert_eq!(
            mac.unprotect(b"header", &mut payload, &invalid_mic, material),
            Err(Ieee802154Error::AuthenticationFailed)
        );
    }

    #[test]
    fn transmit_security_uses_auxiliary_header_counter_and_payload_offset() {
        // Version 1 data frame, security enabled, compressed PAN, short
        // destination and source. The auxiliary header selects ENC-MIC-32.
        let fcf = 0x9849_u16;
        let mut frame = Vec::from(fcf.to_le_bytes());
        frame.push(0x2a);
        frame.extend_from_slice(&0x1234_u16.to_le_bytes());
        frame.extend_from_slice(&0x5678_u16.to_le_bytes());
        frame.extend_from_slice(&0x9abc_u16.to_le_bytes());
        frame.push(5);
        frame.extend_from_slice(&7_u32.to_le_bytes());
        let payload_offset = frame.len();
        frame.extend_from_slice(b"secured");
        frame.extend_from_slice(&[0; 4]);

        let mut mac = Ieee802154Mac::new();
        let protected = mac
            .protect_transmit_frame(
                &frame,
                payload_offset,
                [0x11; 16],
                ExtendedAddress([1, 2, 3, 4, 5, 6, 7, 8]),
            )
            .unwrap();
        assert_eq!(&protected[..payload_offset], &frame[..payload_offset]);
        assert_ne!(&protected[payload_offset..payload_offset + 7], b"secured");
        assert_eq!(protected.len(), frame.len());
        assert_ne!(&protected[protected.len() - 4..], &[0; 4]);
        assert!(mac.events().iter().any(|event| matches!(
            event,
            Ieee802154Event::SecurityProtected {
                level: 5,
                mic_length: 4
            }
        )));
    }

    #[test]
    fn transmit_security_rejects_counter_suppression_and_wrong_offset() {
        let mut frame = vec![0x09, 0x00, 1, 0x25];
        frame.extend_from_slice(b"payload");
        let mut mac = Ieee802154Mac::new();
        assert_eq!(
            mac.protect_transmit_frame(&frame, 4, [0; 16], ExtendedAddress([0; 8])),
            Err(Ieee802154Error::SecurityCounterSuppressed)
        );

        frame[3] = 5;
        frame.splice(4..4, 1_u32.to_le_bytes());
        assert_eq!(
            mac.protect_transmit_frame(&frame, 4, [0; 16], ExtendedAddress([0; 8])),
            Err(Ieee802154Error::InvalidSecurityOffset(4))
        );
    }
}
