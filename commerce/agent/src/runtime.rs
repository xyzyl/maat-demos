//! Agent runtime — the shared state every handler reaches into.

use std::sync::Arc;

use uuid::Uuid;

use crate::config::Config;
use crate::store::AgentStore;

pub struct Runtime {
    pub config: Config,
    pub agent: Arc<AgentStore>,
    /// Maat tenant API key for /v1/verify calls. The agent uses the
    /// same key the store uses (in production they would be distinct).
    pub api_key: String,
    /// Demo customer ID at the store. Provisioned by the setup script
    /// and passed to the agent at startup.
    pub commerce_customer_id: Uuid,
}
