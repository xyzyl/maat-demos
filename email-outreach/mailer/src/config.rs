//! Mailer config (env-driven).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,
    pub maat_gateway_url: String,
    pub maat_api_key: String,
    pub receipt_max_age_seconds: u64,
    pub allowed_domains: Vec<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("OUTREACH_MAILER_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8090".into());
        let database_url = std::env::var("OUTREACH_MAILER_DATABASE_URL")?;
        let maat_gateway_url = std::env::var("MAAT_GATEWAY_URL")?;
        let maat_api_key = std::env::var("MAAT_API_KEY")?;
        let receipt_max_age_seconds = std::env::var("OUTREACH_RECEIPT_MAX_AGE_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        let allowed_domains: Vec<String> = std::env::var("OUTREACH_ALLOWED_DOMAINS")
            .unwrap_or_else(|_| "acme.com".into())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Config {
            bind_addr,
            database_url,
            maat_gateway_url,
            maat_api_key,
            receipt_max_age_seconds,
            allowed_domains,
        })
    }
}
