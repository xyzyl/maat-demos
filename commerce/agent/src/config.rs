//! commerce-agent configuration.

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub gateway_url: String,
    pub store_url: String,
    /// SQLite path for keypair + delegation + activity log.
    pub state_db: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            bind_addr: std::env::var("COMMERCE_AGENT_BIND")
                .unwrap_or_else(|_| "127.0.0.1:9000".to_string()),
            gateway_url: std::env::var("COMMERCE_AGENT_GATEWAY_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string()),
            store_url: std::env::var("COMMERCE_AGENT_STORE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string()),
            state_db: std::env::var("COMMERCE_AGENT_STATE_DB")
                .unwrap_or_else(|_| "agent-state.db".to_string()),
        })
    }
}
