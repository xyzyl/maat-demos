//! Database access for the store's tables.
//!
//! Plain sqlx queries, mirroring the pattern used elsewhere in the
//! workspace. No fancy abstractions — the store's data model is small
//! enough that direct queries are clearer than a repository trait.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;
use async_trait::async_trait;
use maat_resource::ReplayStore;
use maat_sdk_core::SdkError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub sku: String,
    pub name: String,
    pub description: String,
    pub price_cents: i64,
    pub currency: String,
    pub image_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Customer {
    pub id: Uuid,
    pub email: String,
    pub stripe_customer_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Transaction {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub stripe_payment_intent_id: Option<String>,
    pub stripe_error: Option<String>,
    pub maat_receipt_id: Vec<u8>,
    pub maat_delegation_id: Vec<u8>,
    pub rejection_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct PgReplayStore {
    pool: PgPool,
}

impl PgReplayStore {
    pub fn new(pool: PgPool) -> Self {
        PgReplayStore { pool }
    }
}

#[async_trait]
impl ReplayStore for PgReplayStore {
    async fn try_consume(&self, receipt_id: &[u8]) -> Result<bool, SdkError> {
        try_consume_receipt(&self.pool, receipt_id)
            .await
            .map_err(|e| SdkError::Storage(e.to_string()))
    }
}


pub async fn list_products(pool: &PgPool) -> sqlx::Result<Vec<Product>> {
    sqlx::query_as::<_, Product>(
        "SELECT id, sku, name, description, price_cents, currency, image_url, created_at
         FROM products ORDER BY price_cents ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_product(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Product>> {
    sqlx::query_as::<_, Product>(
        "SELECT id, sku, name, description, price_cents, currency, image_url, created_at
         FROM products WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_product(
    pool: &PgPool,
    sku: &str,
    name: &str,
    description: &str,
    price_cents: i64,
    currency: &str,
    image_url: Option<&str>,
) -> sqlx::Result<Product> {
    sqlx::query_as::<_, Product>(
        "INSERT INTO products (sku, name, description, price_cents, currency, image_url)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (sku) DO UPDATE SET
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            price_cents = EXCLUDED.price_cents,
            currency = EXCLUDED.currency,
            image_url = EXCLUDED.image_url
         RETURNING id, sku, name, description, price_cents, currency, image_url, created_at",
    )
    .bind(sku)
    .bind(name)
    .bind(description)
    .bind(price_cents)
    .bind(currency)
    .bind(image_url)
    .fetch_one(pool)
    .await
}

pub async fn get_customer(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Customer>> {
    sqlx::query_as::<_, Customer>(
        "SELECT id, email, stripe_customer_id, created_at
         FROM customers WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn get_customer_by_email(
    pool: &PgPool,
    email: &str,
) -> sqlx::Result<Option<Customer>> {
    sqlx::query_as::<_, Customer>(
        "SELECT id, email, stripe_customer_id, created_at
         FROM customers WHERE email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn create_customer(
    pool: &PgPool,
    email: &str,
    stripe_customer_id: &str,
) -> sqlx::Result<Customer> {
    sqlx::query_as::<_, Customer>(
        "INSERT INTO customers (email, stripe_customer_id) VALUES ($1, $2)
         RETURNING id, email, stripe_customer_id, created_at",
    )
    .bind(email)
    .bind(stripe_customer_id)
    .fetch_one(pool)
    .await
}

pub struct NewTransaction<'a> {
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub amount_cents: i64,
    pub currency: &'a str,
    pub status: &'a str,
    pub stripe_payment_intent_id: Option<&'a str>,
    pub stripe_error: Option<&'a str>,
    pub maat_receipt_id: &'a [u8],
    pub maat_delegation_id: &'a [u8],
    pub rejection_reason: Option<&'a str>,
}

pub async fn record_transaction(
    pool: &PgPool,
    tx: NewTransaction<'_>,
) -> sqlx::Result<Transaction> {
    sqlx::query_as::<_, Transaction>(
        "INSERT INTO transactions
         (customer_id, product_id, amount_cents, currency, status,
          stripe_payment_intent_id, stripe_error,
          maat_receipt_id, maat_delegation_id, rejection_reason)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         RETURNING id, customer_id, product_id, amount_cents, currency, status,
                   stripe_payment_intent_id, stripe_error,
                   maat_receipt_id, maat_delegation_id, rejection_reason, created_at",
    )
    .bind(tx.customer_id)
    .bind(tx.product_id)
    .bind(tx.amount_cents)
    .bind(tx.currency)
    .bind(tx.status)
    .bind(tx.stripe_payment_intent_id)
    .bind(tx.stripe_error)
    .bind(tx.maat_receipt_id)
    .bind(tx.maat_delegation_id)
    .bind(tx.rejection_reason)
    .fetch_one(pool)
    .await
}

pub async fn list_transactions(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<Transaction>> {
    sqlx::query_as::<_, Transaction>(
        "SELECT id, customer_id, product_id, amount_cents, currency, status,
                stripe_payment_intent_id, stripe_error,
                maat_receipt_id, maat_delegation_id, rejection_reason, created_at
         FROM transactions ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Mark a receipt ID as consumed. Returns Ok(true) if newly inserted,
/// Ok(false) if it was already there (replay attempt). The atomic
/// INSERT ... ON CONFLICT DO NOTHING handles the race.
pub async fn try_consume_receipt(pool: &PgPool, receipt_id: &[u8]) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO consumed_receipts (receipt_id) VALUES ($1)
         ON CONFLICT (receipt_id) DO NOTHING",
    )
    .bind(receipt_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}
