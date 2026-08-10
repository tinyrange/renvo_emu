use crate::{CoexistenceEvent, MediumEvent, MediumProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Versioned, portable record of deterministic RF activity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayArtifact {
    /// Artifact schema version. Currently `1`.
    pub schema: u32,
    /// Medium profile needed to reproduce seeded behavior.
    pub profile: MediumProfile,
    /// Append-only medium event stream.
    pub events: Vec<MediumEvent>,
    /// Append-only single-chip coexistence arbitration stream.
    #[serde(default)]
    pub coexistence_events: Vec<CoexistenceEvent>,
}

/// Replay serialization or validation error.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// JSON encoding or decoding failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Artifact uses an unsupported schema.
    #[error("unsupported radio replay schema {0}")]
    UnsupportedSchema(u32),
}

impl ReplayArtifact {
    /// Current replay schema version.
    pub const SCHEMA: u32 = 1;

    /// Constructs an artifact from a completed or partial event stream.
    pub fn new(profile: MediumProfile, events: Vec<MediumEvent>) -> Self {
        Self {
            schema: Self::SCHEMA,
            profile,
            events,
            coexistence_events: Vec::new(),
        }
    }

    /// Attaches coexistence evidence to this artifact.
    #[must_use]
    pub fn with_coexistence_events(mut self, events: Vec<CoexistenceEvent>) -> Self {
        self.coexistence_events = events;
        self
    }

    /// Encodes canonical compact JSON.
    pub fn to_json(&self) -> Result<Vec<u8>, ReplayError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Parses and validates a JSON artifact.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplayError> {
        let artifact: Self = serde_json::from_slice(bytes)?;
        if artifact.schema != Self::SCHEMA {
            return Err(ReplayError::UnsupportedSchema(artifact.schema));
        }
        Ok(artifact)
    }

    /// SHA-256 digest of the canonical JSON representation.
    pub fn digest(&self) -> Result<String, ReplayError> {
        let bytes = self.to_json()?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_round_trip_and_digest_are_stable() {
        let artifact = ReplayArtifact::new(
            MediumProfile {
                seed: 7,
                ..MediumProfile::default()
            },
            Vec::new(),
        );
        let json = artifact.to_json().unwrap();
        assert_eq!(ReplayArtifact::from_json(&json).unwrap(), artifact);
        assert_eq!(artifact.digest().unwrap(), artifact.digest().unwrap());
        assert_eq!(artifact.digest().unwrap().len(), 64);
    }

    #[test]
    fn future_schema_is_rejected() {
        let error = ReplayArtifact::from_json(
            br#"{"schema":2,"profile":{"seed":0,"loss_ppm":0,"capture_threshold_db":10,"default_path_loss_db":40},"events":[]}"#,
        )
        .unwrap_err();
        assert!(matches!(error, ReplayError::UnsupportedSchema(2)));
    }
}
