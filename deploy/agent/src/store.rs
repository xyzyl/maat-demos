//! Sqlite-backed agent state.

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maat::{Delegation, Keypair, PublicKey};
use maat_agent::AgentIdentity;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tokio::sync::{broadcast, RwLock};

const ACTIVITY_CAP: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase { AwaitingDelegation, Ready, Active, Idle }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLevel { Info, Success, Warn, Error }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub at: chrono::DateTime<chrono::Utc>,
    pub level: ActivityLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegationSummary {
    pub id_b64: String,
    pub scope_grants: Vec<String>,
    pub not_after: u64,
    pub constraint_count: usize,
}

pub struct AgentStore {
    pool: SqlitePool,
    identity: AgentIdentity,
    inner: RwLock<Inner>,
    pub activity_tx: broadcast::Sender<ActivityEntry>,
}

struct Inner {
    delegation: Option<Delegation>,
    phase: AgentPhase,
}

impl AgentStore {
    pub async fn open(path: &str) -> anyhow::Result<Arc<Self>> {
        let opts = SqliteConnectOptions::new().filename(path).create_if_missing(true);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(opts).await?;

        sqlx::query("CREATE TABLE IF NOT EXISTS identity (
            id INTEGER PRIMARY KEY CHECK (id=1),
            seed_b64 TEXT NOT NULL,
            public_key_b64 TEXT NOT NULL,
            created_at TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS delegation (
            id INTEGER PRIMARY KEY CHECK (id=1),
            delegation_json TEXT NOT NULL,
            accepted_at TEXT NOT NULL)").execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS activity_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at TEXT NOT NULL, level TEXT NOT NULL, message TEXT NOT NULL)").execute(&pool).await?;

        let identity = match load_identity(&pool).await? {
            Some(id) => id,
            None => { let id = generate(); persist_identity(&pool, &id).await?; id }
        };
        let delegation = load_delegation(&pool).await?;
        let phase = if delegation.is_some() { AgentPhase::Ready } else { AgentPhase::AwaitingDelegation };
        let (tx, _) = broadcast::channel(64);
        Ok(Arc::new(AgentStore {
            pool, identity,
            inner: RwLock::new(Inner { delegation, phase }),
            activity_tx: tx,
        }))
    }

    pub fn public_key(&self) -> &PublicKey { self.identity.public_key() }
    pub fn identity(&self) -> &AgentIdentity { &self.identity }
    pub fn keypair(&self) -> &Keypair { self.identity.as_keypair() }
    pub async fn phase(&self) -> AgentPhase { self.inner.read().await.phase }
    pub async fn delegation(&self) -> Option<Delegation> { self.inner.read().await.delegation.clone() }
    pub async fn delegation_summary(&self) -> Option<DelegationSummary> {
        let g = self.inner.read().await;
        g.delegation.as_ref().map(|d| DelegationSummary {
            id_b64: URL_SAFE_NO_PAD.encode(d.id.0),
            scope_grants: d.scope.grant.clone(),
            not_after: d.not_after,
            constraint_count: d.constraints.len(),
        })
    }

    pub async fn accept_delegation(&self, json: &str) -> Result<(), AcceptError> {
        let d: Delegation = serde_json::from_str(json).map_err(|e| AcceptError::Malformed(e.to_string()))?;
        maat_agent::accept_delegation(&d, self.identity.public_key()).map_err(|e| match e {
            maat_sdk_core::SdkError::Config(m) if m.contains("agent pubkey") => AcceptError::WrongAgent,
            maat_sdk_core::SdkError::Protocol(m) if m.contains("expired") => AcceptError::Expired,
            maat_sdk_core::SdkError::Crypto(m) => AcceptError::SignatureInvalid(m),
            other => AcceptError::Other(other.to_string()),
        })?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT OR REPLACE INTO delegation (id, delegation_json, accepted_at) VALUES (1,?,?)")
            .bind(serde_json::to_string(&d).map_err(|e| AcceptError::Other(e.to_string()))?)
            .bind(now).execute(&self.pool).await.map_err(|e| AcceptError::Other(e.to_string()))?;
        let mut g = self.inner.write().await;
        g.delegation = Some(d);
        g.phase = AgentPhase::Ready;
        drop(g);
        self.record(ActivityLevel::Success, "delegation accepted").await;
        Ok(())
    }

    pub async fn clear_delegation(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM delegation").execute(&self.pool).await?;
        let mut g = self.inner.write().await;
        g.delegation = None;
        g.phase = AgentPhase::AwaitingDelegation;
        drop(g);
        self.record(ActivityLevel::Info, "delegation cleared").await;
        Ok(())
    }

    pub async fn set_phase(&self, p: AgentPhase) { self.inner.write().await.phase = p; }

    pub async fn record(&self, level: ActivityLevel, message: impl Into<String>) {
        let entry = ActivityEntry { at: chrono::Utc::now(), level, message: message.into() };
        let lvl_s = match level {
            ActivityLevel::Info => "info", ActivityLevel::Success => "success",
            ActivityLevel::Warn => "warn", ActivityLevel::Error => "error",
        };
        let _ = sqlx::query("INSERT INTO activity_log (at, level, message) VALUES (?,?,?)")
            .bind(entry.at.to_rfc3339()).bind(lvl_s).bind(&entry.message)
            .execute(&self.pool).await;
        let _ = sqlx::query("DELETE FROM activity_log WHERE id NOT IN (SELECT id FROM activity_log ORDER BY id DESC LIMIT ?)")
            .bind(ACTIVITY_CAP).execute(&self.pool).await;
        let _ = self.activity_tx.send(entry);
    }

    pub async fn recent_activity(&self, limit: i64) -> Vec<ActivityEntry> {
        let rows = sqlx::query("SELECT at, level, message FROM activity_log ORDER BY id DESC LIMIT ?")
            .bind(limit).fetch_all(&self.pool).await.unwrap_or_default();
        let mut out: Vec<ActivityEntry> = rows.into_iter().filter_map(|r| {
            let at_s: String = r.try_get("at").ok()?;
            let lvl_s: String = r.try_get("level").ok()?;
            let msg: String = r.try_get("message").ok()?;
            let at = chrono::DateTime::parse_from_rfc3339(&at_s).ok()?.with_timezone(&chrono::Utc);
            let level = match lvl_s.as_str() {
                "success" => ActivityLevel::Success, "warn" => ActivityLevel::Warn,
                "error" => ActivityLevel::Error, _ => ActivityLevel::Info,
            };
            Some(ActivityEntry { at, level, message: msg })
        }).collect();
        out.reverse(); out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error("malformed JSON: {0}")] Malformed(String),
    #[error("delegation agent pubkey doesn't match this agent")] WrongAgent,
    #[error("delegation already expired")] Expired,
    #[error("signature invalid: {0}")] SignatureInvalid(String),
    #[error("{0}")] Other(String),
}

fn generate() -> AgentIdentity {
    let mut s = [0u8; 32]; rand::thread_rng().fill_bytes(&mut s);
    AgentIdentity::from_seed(s)
}

async fn load_identity(pool: &SqlitePool) -> anyhow::Result<Option<AgentIdentity>> {
    let row = sqlx::query("SELECT seed_b64 FROM identity WHERE id=1").fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let b: String = row.try_get("seed_b64")?;
    let v = URL_SAFE_NO_PAD.decode(&b).map_err(|e| anyhow::anyhow!(e))?;
    if v.len() != 32 { anyhow::bail!("bad seed length") }
    let mut s = [0u8; 32]; s.copy_from_slice(&v);
    Ok(Some(AgentIdentity::from_seed(s)))
}

async fn persist_identity(pool: &SqlitePool, id: &AgentIdentity) -> anyhow::Result<()> {
    let s = URL_SAFE_NO_PAD.encode(id.secret_seed());
    let p = URL_SAFE_NO_PAD.encode(&id.public_key().key_data);
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO identity (id, seed_b64, public_key_b64, created_at) VALUES (1,?,?,?)")
        .bind(s).bind(p).bind(now).execute(pool).await?;
    Ok(())
}

async fn load_delegation(pool: &SqlitePool) -> anyhow::Result<Option<Delegation>> {
    let row = sqlx::query("SELECT delegation_json FROM delegation WHERE id=1").fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let j: String = row.try_get("delegation_json")?;
    Ok(Some(serde_json::from_str(&j)?))
}
