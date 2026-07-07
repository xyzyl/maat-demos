use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub bind_addr: String,
    pub agent_db_path: String,
    pub gateway_url: String,
    pub api_key: String,
    pub mailer_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("OUTREACH_AGENT_BIND")
            .unwrap_or_else(|_| "0.0.0.0:9090".into());
        let agent_db_path = std::env::var("OUTREACH_AGENT_DB_PATH")
            .unwrap_or_else(|_| "./outreach-agent-state.db".into());
        let gateway_url = std::env::var("MAAT_GATEWAY_URL")?;
        let api_key = std::env::var("MAAT_API_KEY")?;
        let mailer_url = std::env::var("OUTREACH_MAILER_URL")?;
        Ok(Config {
            bind_addr,
            agent_db_path,
            gateway_url,
            api_key,
            mailer_url,
        })
    }
}
