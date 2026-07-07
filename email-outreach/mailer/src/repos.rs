//! Postgres-backed replay store + sends audit table.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maat_resource::ReplayStore;
use maat_sdk_core::SdkError;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PgReplayStore {
    pool: PgPool,
}

impl PgReplayStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReplayStore for PgReplayStore {
    async fn try_consume(&self, receipt_id: &[u8]) -> Result<bool, SdkError> {
        let result = sqlx::query(
            "INSERT INTO consumed_receipts (receipt_id) VALUES ($1)
             ON CONFLICT (receipt_id) DO NOTHING",
        )
        .bind(receipt_id)
        .execute(&self.pool)
        .await
        .map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(result.rows_affected() == 1)
    }
}

// ─── Sends audit table ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SendRecord {
    pub id: Uuid,
    pub recipients_json: String,
    pub content_hash_hex: String,
    pub status: String,
    pub rejection_reason: Option<String>,
    pub maat_receipt_id: Vec<u8>,
    pub maat_delegation_id: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

pub struct NewSend<'a> {
    pub recipients_json: &'a str,
    pub content_hash_hex: &'a str,
    pub status: &'a str,
    pub rejection_reason: Option<&'a str>,
    pub maat_receipt_id: &'a [u8],
    pub maat_delegation_id: &'a [u8],
}

pub async fn record_send(pool: &PgPool, s: NewSend<'_>) -> sqlx::Result<SendRecord> {
    let row = sqlx::query(
        "INSERT INTO sends
            (recipients_json, content_hash_hex, status, rejection_reason,
             maat_receipt_id, maat_delegation_id)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, recipients_json, content_hash_hex, status, rejection_reason,
                   maat_receipt_id, maat_delegation_id, created_at",
    )
    .bind(s.recipients_json)
    .bind(s.content_hash_hex)
    .bind(s.status)
    .bind(s.rejection_reason)
    .bind(s.maat_receipt_id)
    .bind(s.maat_delegation_id)
    .fetch_one(pool)
    .await?;
    Ok(SendRecord {
        id: row.get("id"),
        recipients_json: row.get("recipients_json"),
        content_hash_hex: row.get("content_hash_hex"),
        status: row.get("status"),
        rejection_reason: row.get("rejection_reason"),
        maat_receipt_id: row.get("maat_receipt_id"),
        maat_delegation_id: row.get("maat_delegation_id"),
        created_at: row.get("created_at"),
    })
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<SendRecord>> {
    let rows = sqlx::query(
        "SELECT id, recipients_json, content_hash_hex, status, rejection_reason,
                maat_receipt_id, maat_delegation_id, created_at
         FROM sends ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SendRecord {
            id: r.get("id"),
            recipients_json: r.get("recipients_json"),
            content_hash_hex: r.get("content_hash_hex"),
            status: r.get("status"),
            rejection_reason: r.get("rejection_reason"),
            maat_receipt_id: r.get("maat_receipt_id"),
            maat_delegation_id: r.get("maat_delegation_id"),
            created_at: r.get("created_at"),
        })
        .collect())
}
