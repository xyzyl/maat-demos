//! Commerce-store configuration.

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: String,
    pub database_url: String,

    /// Stripe secret key (sk_test_* during development).
    pub stripe_secret_key: String,

    /// Maat gateway base URL — store fetches the executor key from here
    /// once at startup for receipt verification.
    pub maat_gateway_url: String,

    /// Maat tenant API key — used to authenticate the store's
    /// /v1/executor-key fetch.
    pub maat_api_key: String,

    /// How fresh a receipt must be (seconds since executed_at) before
    /// we'll honor it. Defends against late replay.
    pub receipt_max_age_seconds: u64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("COMMERCE_STORE_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8090".to_string());

        let database_url = std::env::var("COMMERCE_STORE_DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("COMMERCE_STORE_DATABASE_URL is required"))?;

        let stripe_secret_key = std::env::var("STRIPE_SECRET_KEY")
            .map_err(|_| anyhow::anyhow!("STRIPE_SECRET_KEY is required"))?;
        if !stripe_secret_key.starts_with("sk_") {
            anyhow::bail!("STRIPE_SECRET_KEY must start with 'sk_' (test or live)");
        }

        let maat_gateway_url = std::env::var("MAAT_GATEWAY_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());

        let maat_api_key = std::env::var("MAAT_API_KEY")
            .map_err(|_| anyhow::anyhow!("MAAT_API_KEY is required"))?;

        let receipt_max_age_seconds = std::env::var("COMMERCE_STORE_RECEIPT_MAX_AGE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);

        Ok(Config {
            bind_addr,
            database_url,
            stripe_secret_key,
            maat_gateway_url,
            maat_api_key,
            receipt_max_age_seconds,
        })
    }
}
