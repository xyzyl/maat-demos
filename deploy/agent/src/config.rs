use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub bind_addr: String,
    pub agent_db_path: String,
    pub gateway_url: String,
    pub api_key: String,
    pub runner_url: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("DEPLOY_AGENT_BIND").unwrap_or_else(|_| "0.0.0.0:9091".into());
        let agent_db_path = std::env::var("DEPLOY_AGENT_DB_PATH").unwrap_or_else(|_| "./deploy-agent-state.db".into());
        let gateway_url = std::env::var("MAAT_GATEWAY_URL")?;
        let api_key = std::env::var("MAAT_API_KEY")?;
        let runner_url = std::env::var("DEPLOY_RUNNER_URL")?;
        Ok(Config { bind_addr, agent_db_path, gateway_url, api_key, runner_url })
    }
}
