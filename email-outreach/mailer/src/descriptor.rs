//! Outreach action descriptor.
//!
//! `RecipientClaim` is the agent's structured commitment to a specific
//! email send: a list of recipient addresses and the SHA-256 hash of
//! the message body. The mailer enforces that the actual request body
//! matches both (recipients verbatim, body hash exactly).
//!
//! The descriptor is defined IN-CRATE on each side (mailer and agent)
//! and the wire format is JSON. The two sides interoperate through
//! the shape of the JSON, not via a shared Rust type. This mirrors
//! production patterns where agent and resource are separately
//! deployed services that communicate over wire formats.

use maat_sdk_core::{ActionDescriptor, DescriptorError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecipientClaim {
    /// Email addresses the agent is authorized to contact in this action.
    pub recipients: Vec<String>,
    /// SHA-256 of the email body bytes (hex-encoded for human readability
    /// in audit logs).
    pub content_hash_hex: String,
}

impl ActionDescriptor for RecipientClaim {
    fn to_action_value(&self) -> Result<Vec<u8>, DescriptorError> {
        serde_json::to_vec(self).map_err(|e| DescriptorError::Encode(e.to_string()))
    }

    fn from_action_value(bytes: &[u8]) -> Result<Self, DescriptorError> {
        serde_json::from_slice(bytes).map_err(|e| DescriptorError::Decode(e.to_string()))
    }
}

/// Hex-encode a 32-byte SHA-256 digest.
pub fn hex_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let original = RecipientClaim {
            recipients: vec!["a@x.com".into(), "b@x.com".into()],
            content_hash_hex: "abcd".repeat(16),
        };
        let bytes = original.to_action_value().unwrap();
        let back = RecipientClaim::from_action_value(&bytes).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn rejects_garbage() {
        assert!(RecipientClaim::from_action_value(b"not json").is_err());
    }
}
