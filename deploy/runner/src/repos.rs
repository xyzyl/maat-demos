use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maat_resource::ReplayStore;
use maat_sdk_core::SdkError;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PgReplayStore { pool: PgPool }
impl PgReplayStore { pub fn new(pool: PgPool) -> Self { Self { pool } } }

#[async_trait]
impl ReplayStore for PgReplayStore {
    async fn try_consume(&self, receipt_id: &[u8]) -> Result<bool, SdkError> {
        let r = sqlx::query(
            "INSERT INTO consumed_receipts (receipt_id) VALUES ($1)
             ON CONFLICT (receipt_id) DO NOTHING")
            .bind(receipt_id)
            .execute(&self.pool)
            .await.map_err(|e| SdkError::Storage(e.to_string()))?;
        Ok(r.rows_affected() == 1)
    }
}

#[derive(Debug, Clone)]
pub struct DeployRecord {
    pub id: Uuid,
    pub environment: String,
    pub service: String,
    pub commit_hash: String,
    pub status: String,
    pub rejection_reason: Option<String>,
    pub maat_receipt_id: Vec<u8>,
    pub maat_delegation_id: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

pub struct NewDeploy<'a> {
    pub environment: &'a str,
    pub service: &'a str,
    pub commit_hash: &'a str,
    pub status: &'a str,
    pub rejection_reason: Option<&'a str>,
    pub maat_receipt_id: &'a [u8],
    pub maat_delegation_id: &'a [u8],
}

pub async fn record(pool: &PgPool, d: NewDeploy<'_>) -> sqlx::Result<DeployRecord> {
    let row = sqlx::query(
        "INSERT INTO deploys
            (environment, service, commit_hash, status, rejection_reason,
             maat_receipt_id, maat_delegation_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         RETURNING id, environment, service, commit_hash, status, rejection_reason,
                   maat_receipt_id, maat_delegation_id, created_at")
        .bind(d.environment).bind(d.service).bind(d.commit_hash)
        .bind(d.status).bind(d.rejection_reason)
        .bind(d.maat_receipt_id).bind(d.maat_delegation_id)
        .fetch_one(pool).await?;
    Ok(DeployRecord {
        id: row.get("id"), environment: row.get("environment"),
        service: row.get("service"), commit_hash: row.get("commit_hash"),
        status: row.get("status"), rejection_reason: row.get("rejection_reason"),
        maat_receipt_id: row.get("maat_receipt_id"),
        maat_delegation_id: row.get("maat_delegation_id"),
        created_at: row.get("created_at"),
    })
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<DeployRecord>> {
    let rows = sqlx::query(
        "SELECT id, environment, service, commit_hash, status, rejection_reason,
                maat_receipt_id, maat_delegation_id, created_at
         FROM deploys ORDER BY created_at DESC LIMIT $1")
        .bind(limit).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| DeployRecord {
        id: r.get("id"), environment: r.get("environment"),
        service: r.get("service"), commit_hash: r.get("commit_hash"),
        status: r.get("status"), rejection_reason: r.get("rejection_reason"),
        maat_receipt_id: r.get("maat_receipt_id"),
        maat_delegation_id: r.get("maat_delegation_id"),
        created_at: r.get("created_at"),
    }).collect())
}
