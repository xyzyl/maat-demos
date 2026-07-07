//! Shared mailer state.

use std::time::Duration;

use maat_resource::{ExecutorKey, ReceiptValidator};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;
use crate::descriptor::RecipientClaim;
use crate::repos::PgReplayStore;

pub type OutreachValidator = ReceiptValidator<RecipientClaim, PgReplayStore>;

pub struct AppState {
    pub config: Config,
    pub pool: PgPool,
    pub validator: OutreachValidator,
}

impl AppState {
    pub async fn new(config: Config, executor_key: ExecutorKey) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .map_err(|e| anyhow::anyhow!("postgres connect failed: {}", e))?;

        let replay = PgReplayStore::new(pool.clone());
        let max_age = config.receipt_max_age_seconds;
        let validator = ReceiptValidator::<RecipientClaim, _>::new(executor_key, replay)
            .with_max_age(Duration::from_secs(max_age));

        Ok(AppState {
            config,
            pool,
            validator,
        })
    }
}
