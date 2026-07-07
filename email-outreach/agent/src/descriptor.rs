//! Outreach agent action descriptor.
//!
//! Defined in-crate; the agent and mailer interoperate via the wire
//! shape of the JSON, not a shared Rust type.

use maat_sdk_core::{ActionDescriptor, DescriptorError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecipientClaim {
    pub recipients: Vec<String>,
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

pub fn hex_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}
