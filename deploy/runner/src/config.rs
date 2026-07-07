use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub maat_gateway_url: String,
    pub maat_api_key: String,
    pub receipt_max_age_seconds: u64,
    pub allowed_environments: Vec<String>,
    pub allowed_services: Vec<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("DEPLOY_RUNNER_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8091".into());
        let database_url = std::env::var("DEPLOY_RUNNER_DATABASE_URL")?;
        let maat_gateway_url = std::env::var("MAAT_GATEWAY_URL")?;
        let maat_api_key = std::env::var("MAAT_API_KEY")?;
        let receipt_max_age_seconds = std::env::var("DEPLOY_RECEIPT_MAX_AGE_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30); // tighter for deploys
        let allowed_environments: Vec<String> = std::env::var("DEPLOY_ALLOWED_ENVIRONMENTS")
            .unwrap_or_else(|_| "staging,production".into())
            .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let allowed_services: Vec<String> = std::env::var("DEPLOY_ALLOWED_SERVICES")
            .unwrap_or_else(|_| "api,worker,web".into())
            .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        Ok(Config {
            bind_addr, database_url, maat_gateway_url, maat_api_key,
            receipt_max_age_seconds, allowed_environments, allowed_services,
        })
    }
}
