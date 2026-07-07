//! Shared store state.
//!
//! Slice 10a: the cached executor `PublicKey` is gone. In its place is
//! a `ReceiptValidator` that owns the executor key, the postgres-backed
//! replay store, and the freshness window. Handlers call
//! `state.validator.validate(receipt_bytes).await` instead of
//! reimplementing the six fail-closed checks inline.


use std::time::Duration;

use maat::ValueClaim;
use maat_resource::{ExecutorKey, ReceiptValidator};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;
use crate::repos::PgReplayStore;
use crate::stripe_client::StripeClient;

/// Receipt validator instantiated once at startup. Generic over
/// `ValueClaim` because that's the descriptor shape commerce uses;
/// other resources would parameterize differently.
pub type CommerceValidator = ReceiptValidator<ValueClaim, PgReplayStore>;

pub struct AppState {
    pub config: Config,
    pub pool: PgPool,
    pub stripe: StripeClient,
    /// Slice 10a: the SDK's receipt validator replaces the bare
    /// executor key + ad-hoc verification logic that used to live
    /// in handlers/checkout.rs.
    pub validator: CommerceValidator,
}

impl AppState {
    pub async fn new(config: Config, executor_key: ExecutorKey) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .map_err(|e| anyhow::anyhow!("postgres connect failed: {}", e))?;

        let stripe = StripeClient::new(&config.stripe_secret_key);

        let max_age = config.receipt_max_age_seconds;
        let replay = PgReplayStore::new(pool.clone());
        let validator = ReceiptValidator::<ValueClaim, _>::new(executor_key, replay)
            .with_max_age(Duration::from_secs(max_age));
        Ok(AppState {
            config,
            pool,
            stripe,
            validator,
        })
    }
}
