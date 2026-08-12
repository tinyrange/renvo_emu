/// Native Wi-Fi transmit result published by the hardware completion record.
///
/// The values are recovered from the pinned Apache-2.0 ESP32 Wi-Fi libraries.
/// They are hardware status fields consumed by the guest's LMAC; retry policy
/// remains entirely in guest firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EspWifiTxOutcome {
    /// The frame completed, including any required peer acknowledgement.
    Success,
    /// The MAC could not obtain the shared RF path for this transmit attempt.
    ///
    /// Guest LMAC dispatches status four through its transmit-error path, so
    /// firmware remains responsible for deciding whether to retry.
    TransmitError,
    /// The frame required an 802.11 ACK and none arrived before the timeout.
    AckTimeout,
}

impl EspWifiTxOutcome {
    pub(crate) const fn status(self) -> u32 {
        match self {
            Self::Success => 0,
            Self::TransmitError => 4,
            Self::AckTimeout => 5,
        }
    }
}

/// One firmware-programmed native Wi-Fi MAC crypto-table entry.
///
/// The pinned C6 and S3 Apache-2.0 Wi-Fi libraries use the same forty-byte
/// entry shape: two match/control words followed by up to 32 key bytes. The
/// cipher, interface, key-ID and peer match fields retain their native hardware
/// encoding; exposing the recovered word avoids inventing a guest-facing HLE
/// key API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EspWifiCryptoKeyEntry {
    /// Zero-based hardware table slot selected by firmware.
    pub slot: u8,
    /// First firmware-programmed match word.
    pub match_low: u32,
    /// Second match word plus native cipher/interface control fields.
    pub control: u32,
    /// Complete fixed-width native key payload area.
    pub key: [u8; 32],
}

impl EspWifiCryptoKeyEntry {
    const fn cipher(self) -> u8 {
        ((self.control >> 18) & 7) as u8
    }

    const fn interface(self) -> u8 {
        ((self.control >> 24) & 3) as u8
    }

    const fn key_id(self) -> u8 {
        ((self.control >> 30) & 3) as u8
    }

    fn peer(self) -> [u8; 6] {
        let low = self.match_low.to_le_bytes();
        let high = (self.control as u16).to_le_bytes();
        [low[0], low[1], low[2], low[3], high[0], high[1]]
    }

    fn ccmp_key(self) -> Option<[u8; 16]> {
        (self.cipher() == 3).then(|| self.key[..16].try_into().expect("fixed CCMP key width"))
    }
}

pub(crate) struct EspWifiCcmpTxSelector {
    pub(crate) receiver: [u8; 6],
    pub(crate) transmitter: [u8; 6],
    pub(crate) key_id: u8,
}

impl EspWifiCcmpTxSelector {
    pub(crate) fn parse(frame: &[u8]) -> Result<Self, String> {
        let frame_control = u16::from_le_bytes(
            frame
                .get(..2)
                .ok_or_else(|| "hardware-protected TX is missing frame control".to_owned())?
                .try_into()
                .expect("checked frame-control width"),
        );
        let frame_type = frame_control & 0x000c;
        if !matches!(frame_type, 0x0000 | 0x0008) || frame_control & 0x4000 == 0 {
            return Err(format!(
                "hardware-protected TX has unsupported frame control {frame_control:#06x}"
            ));
        }
        let address_four = frame_control & 0x0300 == 0x0300;
        let qos = frame_type == 0x0008 && frame_control & 0x0080 != 0;
        let high_throughput_control = qos && frame_control & 0x8000 != 0;
        let header_length = 24
            + usize::from(address_four) * 6
            + usize::from(qos) * 2
            + usize::from(high_throughput_control) * 4;
        let receiver: [u8; 6] = frame
            .get(4..10)
            .ok_or_else(|| "hardware-protected TX is missing its receiver address".to_owned())?
            .try_into()
            .expect("checked receiver width");
        let transmitter: [u8; 6] = frame
            .get(10..16)
            .ok_or_else(|| "hardware-protected TX is missing its transmitter address".to_owned())?
            .try_into()
            .expect("checked transmitter width");
        let ccmp = frame
            .get(header_length..header_length + 8)
            .ok_or_else(|| "hardware-protected TX is missing its CCMP header".to_owned())?;
        if ccmp[2] != 0 || ccmp[3] & 0x3f != 0x20 {
            return Err("hardware-protected TX has an invalid CCMP extended-IV header".to_owned());
        }
        frame
            .len()
            .checked_sub(8)
            .filter(|end| *end >= header_length + 8)
            .ok_or_else(|| "hardware-protected TX is missing its eight-byte MIC area".to_owned())?;
        Ok(Self {
            receiver,
            transmitter,
            key_id: ccmp[3] >> 6,
        })
    }
}

pub(crate) fn select_esp_wifi_ccmp_tx_key(
    entries: impl IntoIterator<Item = EspWifiCryptoKeyEntry>,
    selector: &EspWifiCcmpTxSelector,
    interface: u8,
) -> Result<[u8; 16], String> {
    let mut selected = None;
    for entry in entries {
        if entry.interface() != interface
            || entry.key_id() != selector.key_id
            || entry.peer() != selector.receiver
        {
            continue;
        }
        let Some(key) = entry.ccmp_key() else {
            continue;
        };
        if let Some((previous, _)) = selected {
            return Err(format!(
                "hardware-protected TX ambiguously matches crypto slots {previous} and {}",
                entry.slot
            ));
        }
        selected = Some((entry.slot, key));
    }
    selected.map(|(_, key)| key).ok_or_else(|| {
        format!(
            "hardware-protected TX has no CCMP key for interface {interface}, key ID {}, peer {:02x?}",
            selector.key_id, selector.receiver
        )
    })
}

#[cfg(test)]
mod tests {
    use super::EspWifiTxOutcome;

    #[test]
    fn native_completion_statuses_match_guest_lmac_dispatch() {
        assert_eq!(EspWifiTxOutcome::Success.status(), 0);
        assert_eq!(EspWifiTxOutcome::TransmitError.status(), 4);
        assert_eq!(EspWifiTxOutcome::AckTimeout.status(), 5);
    }
}
