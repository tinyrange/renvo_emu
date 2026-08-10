use crate::TargetId;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use thiserror::Error;

/// SHA-256 of Espressif's pinned ESP32-C6 revision-zero mask-ROM ELF.
pub const ESP32C6_RADIO_ROM_SHA256: &str =
    "788e1d38724aeb8fd974fa10c4a7b089c02627d35342ce84b9e0b12b239f3551";

/// SHA-256 of Espressif's pinned ESP32-S3 revision-zero mask-ROM ELF.
pub const ESP32S3_RADIO_ROM_SHA256: &str =
    "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd";

/// Failure to establish the exact real-ROM prerequisite for native radio use.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EspRadioRomError {
    /// Only the two radio targets have a pinned ROM contract.
    #[error("target {0} has no pinned ESP radio ROM contract")]
    UnsupportedTarget(TargetId),
    /// The supplied file is not the immutable pinned mask-ROM ELF.
    #[error(
        "{target} native/radio execution requires the pinned real mask-ROM ELF: expected SHA-256 {expected}, got {actual}"
    )]
    Digest {
        /// Target whose ROM was requested.
        target: TargetId,
        /// Pinned digest from the qualification contract.
        expected: &'static str,
        /// Digest of the caller-provided bytes.
        actual: String,
    },
}

/// Verifies the exact immutable ROM ELF required for native C6/S3 radio use.
///
/// This check operates on the original file bytes before ELF parsing so a
/// reconstructed or partially copied image cannot satisfy the requirement.
pub fn verify_esp_radio_rom(target: TargetId, bytes: &[u8]) -> Result<(), EspRadioRomError> {
    let expected = match target {
        TargetId::Esp32c6 => ESP32C6_RADIO_ROM_SHA256,
        TargetId::Esp32s3 => ESP32S3_RADIO_ROM_SHA256,
        _ => return Err(EspRadioRomError::UnsupportedTarget(target)),
    };
    let digest = Sha256::digest(bytes);
    let mut actual = String::with_capacity(64);
    for byte in digest {
        write!(&mut actual, "{byte:02x}").expect("writing a SHA-256 digest to String cannot fail");
    }
    if actual == expected {
        Ok(())
    } else {
        Err(EspRadioRomError::Digest {
            target,
            expected,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_substituted_rom_bytes_with_a_stable_diagnostic() {
        let error = verify_esp_radio_rom(TargetId::Esp32c6, b"not a mask ROM").unwrap_err();
        let diagnostic = error.to_string();
        let EspRadioRomError::Digest {
            target,
            expected,
            ref actual,
        } = error
        else {
            panic!("expected a digest error")
        };
        assert_eq!(target, TargetId::Esp32c6);
        assert_eq!(expected, ESP32C6_RADIO_ROM_SHA256);
        assert_eq!(actual.len(), 64);
        assert!(diagnostic.contains("requires the pinned real mask-ROM ELF"));
    }

    #[test]
    fn rejects_targets_without_a_radio_rom_contract() {
        assert_eq!(
            verify_esp_radio_rom(TargetId::Rp2040, &[]),
            Err(EspRadioRomError::UnsupportedTarget(TargetId::Rp2040))
        );
    }
}
