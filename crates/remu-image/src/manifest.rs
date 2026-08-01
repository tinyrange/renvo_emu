use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Container format of an official firmware artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirmwareArtifactFormat {
    /// Microsoft UF2 block container.
    Uf2,
    /// Espressif merged flash binary.
    EspBin,
    /// Intel HEX with absolute device addresses.
    IntelHex,
    /// Addressless raw flash bytes.
    RawBin,
}

/// One immutable artifact published by a firmware project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficialFirmwareArtifact {
    /// Stable Renvo Emulator identifier.
    pub id: String,
    /// Board name used in qualification reports.
    pub board: String,
    /// CPU execution profile selected by the artifact.
    pub profile: String,
    /// Canonical HTTPS download URL.
    pub url: String,
    /// Cache filename.
    pub filename: String,
    /// Expected lowercase SHA-256.
    pub sha256: String,
    /// Firmware container format.
    pub format: FirmwareArtifactFormat,
    /// True when this exact artifact is executed by the final gate.
    #[serde(default)]
    pub primary: bool,
}

/// Versioned set of official firmware artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficialFirmwareSuite {
    /// Manifest schema identifier.
    pub schema: String,
    /// Stable suite name.
    pub name: String,
    /// Upstream release version.
    pub version: String,
    /// Upstream release date in ISO form.
    pub released: String,
    /// Required artifacts.
    pub artifacts: Vec<OfficialFirmwareArtifact>,
}

/// A local artifact whose content matches the official manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedFirmwareArtifact {
    /// Stable artifact ID.
    pub id: String,
    /// Local path.
    pub path: PathBuf,
    /// File byte length.
    pub size: u64,
    /// Verified SHA-256.
    pub sha256: String,
    /// Declared container format.
    pub format: FirmwareArtifactFormat,
    /// Final-gate status.
    pub primary: bool,
}

impl OfficialFirmwareSuite {
    /// Parses and validates a TOML manifest.
    pub fn from_toml(text: &str) -> Result<Self, FirmwareManifestError> {
        let suite: Self = toml::from_str(text)?;
        suite.validate()?;
        Ok(suite)
    }

    /// Loads and validates a TOML manifest.
    pub fn read(path: &Path) -> Result<Self, FirmwareManifestError> {
        let text = fs::read_to_string(path).map_err(|source| FirmwareManifestError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text)
    }

    /// Checks schema, identifiers, URLs, filenames and hashes.
    pub fn validate(&self) -> Result<(), FirmwareManifestError> {
        if self.schema != "remu.official-firmware.v1" {
            return Err(FirmwareManifestError::Schema(self.schema.clone()));
        }
        if self.name.trim().is_empty()
            || self.version.trim().is_empty()
            || self.released.trim().is_empty()
            || self.artifacts.is_empty()
        {
            return Err(FirmwareManifestError::Invalid(
                "suite metadata and artifact list must not be empty".to_owned(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut filenames = BTreeSet::new();
        for artifact in &self.artifacts {
            if !safe_id(&artifact.id) {
                return Err(FirmwareManifestError::Invalid(format!(
                    "unsafe artifact ID {:?}",
                    artifact.id
                )));
            }
            if !ids.insert(&artifact.id) {
                return Err(FirmwareManifestError::DuplicateId(artifact.id.clone()));
            }
            if artifact.board.trim().is_empty() || artifact.profile.trim().is_empty() {
                return Err(FirmwareManifestError::Invalid(format!(
                    "artifact {:?} has empty board or profile",
                    artifact.id
                )));
            }
            if !safe_filename(&artifact.filename) {
                return Err(FirmwareManifestError::Invalid(format!(
                    "unsafe artifact filename {:?}",
                    artifact.filename
                )));
            }
            if !filenames.insert(&artifact.filename) {
                return Err(FirmwareManifestError::DuplicateFilename(
                    artifact.filename.clone(),
                ));
            }
            if !artifact.url.starts_with("https://") || !artifact.url.ends_with(&artifact.filename)
            {
                return Err(FirmwareManifestError::Invalid(format!(
                    "artifact {:?} URL must be HTTPS and end with its filename",
                    artifact.id
                )));
            }
            if artifact.sha256.len() != 64
                || !artifact
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(FirmwareManifestError::Invalid(format!(
                    "artifact {:?} has invalid lowercase SHA-256",
                    artifact.id
                )));
            }
        }
        Ok(())
    }

    /// Verifies every file in `directory` against the manifest.
    pub fn verify_directory(
        &self,
        directory: &Path,
    ) -> Result<Vec<VerifiedFirmwareArtifact>, FirmwareManifestError> {
        self.validate()?;
        let mut verified = Vec::with_capacity(self.artifacts.len());
        for artifact in &self.artifacts {
            let path = directory.join(&artifact.filename);
            let bytes = fs::read(&path).map_err(|source| FirmwareManifestError::Io {
                path: path.clone(),
                source,
            })?;
            let actual = hex::encode(Sha256::digest(&bytes));
            if actual != artifact.sha256 {
                return Err(FirmwareManifestError::Hash {
                    id: artifact.id.clone(),
                    expected: artifact.sha256.clone(),
                    actual,
                });
            }
            verified.push(VerifiedFirmwareArtifact {
                id: artifact.id.clone(),
                path,
                size: u64::try_from(bytes.len()).expect("file length fits u64"),
                sha256: actual,
                format: artifact.format,
                primary: artifact.primary,
            });
        }
        Ok(verified)
    }
}

/// Official firmware manifest or cache validation failure.
#[derive(Debug, Error)]
pub enum FirmwareManifestError {
    #[error("unsupported official firmware manifest schema {0:?}")]
    Schema(String),
    #[error("invalid official firmware manifest: {0}")]
    Invalid(String),
    #[error("duplicate official firmware artifact ID {0:?}")]
    DuplicateId(String),
    #[error("duplicate official firmware filename {0:?}")]
    DuplicateFilename(String),
    #[error("artifact {id:?} SHA-256 mismatch: expected {expected}, got {actual}")]
    Hash {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("cannot access firmware artifact {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse official firmware TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_filename(value: &str) -> bool {
    safe_id(value) && !value.starts_with('.') && !value.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_hashes_a_cache() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"official bytes";
        fs::write(directory.path().join("board.uf2"), bytes).unwrap();
        let hash = hex::encode(Sha256::digest(bytes));
        let suite = OfficialFirmwareSuite {
            schema: "remu.official-firmware.v1".to_owned(),
            name: "test".to_owned(),
            version: "1".to_owned(),
            released: "2026-01-01".to_owned(),
            artifacts: vec![OfficialFirmwareArtifact {
                id: "board".to_owned(),
                board: "Board".to_owned(),
                profile: "cpu".to_owned(),
                url: "https://example.invalid/board.uf2".to_owned(),
                filename: "board.uf2".to_owned(),
                sha256: hash,
                format: FirmwareArtifactFormat::Uf2,
                primary: true,
            }],
        };
        let verified = suite.verify_directory(directory.path()).unwrap();
        assert_eq!(verified[0].size, bytes.len() as u64);
    }
}
