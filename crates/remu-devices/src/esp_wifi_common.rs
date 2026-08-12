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
