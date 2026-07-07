use maat_sdk_core::{ActionDescriptor, DescriptorError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetClaim {
    pub environment: String,
    pub commit_hash: String,
    pub service: String,
}

impl ActionDescriptor for TargetClaim {
    fn to_action_value(&self) -> Result<Vec<u8>, DescriptorError> {
        serde_json::to_vec(self).map_err(|e| DescriptorError::Encode(e.to_string()))
    }
    fn from_action_value(bytes: &[u8]) -> Result<Self, DescriptorError> {
        serde_json::from_slice(bytes).map_err(|e| DescriptorError::Decode(e.to_string()))
    }
}
