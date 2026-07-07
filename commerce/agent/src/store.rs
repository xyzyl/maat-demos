//! Agent state machine and local persistence.
//!
//! Three sqlite tables in a single file (`agent-state.db`):
//!   - identity       — single row, the agent's keypair (stored as 32-byte seed)
//!   - delegation     — single row when present, the active delegation
//!   - activity_log   — ring buffer of recent actions (last 200)
//!
//! Slice 10a: keypair is now wrapped in `AgentIdentity` from the SDK.
//! Persistence still uses the raw 32-byte seed — `AgentIdentity` exposes
//! `secret_seed()`/`from_seed()` round-trip for exactly this case.

use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maat::{Delegation, Keypair, PublicKey};
use maat_agent::AgentIdentity;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tokio::sync::{broadcast, RwLock};

const ACTIVITY_LOG_CAP: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    AwaitingDelegation,
    Ready,
    Active,
    Idle,
}

#[derive(Debug, Clone, Serialize)]
pub struct DelegationSummary {
    pub id_b64: String,
    pub scope_grants: Vec<String>,
    pub not_after: u64,
    pub constraint_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub at: chrono::DateTime<chrono::Utc>,
    pub level: ActivityLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityLevel {
    Info,
    Success,
    Warn,
    Error,
}

pub struct AgentStore {
    pool: SqlitePool,
    identity: AgentIdentity,
    inner: RwLock<Inner>,
    /// Sender used by `record_activity` to fan out new entries to any
    /// SSE subscribers in the UI.
    pub activity_tx: broadcast::Sender<ActivityEntry>,
}

struct Inner {
    delegation: Option<Delegation>,
    phase: AgentPhase,
    current_goal: Option<String>,
}

impl AgentStore {
    /// Open or create the agent-state.db, generate an identity on first
    /// run, load any persisted delegation. Sets phase based on what's
    /// present.
    pub async fn open(path: &str) -> anyhow::Result<Arc<Self>> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;

        // Apply schema (idempotent).
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS identity (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                seed_b64 TEXT NOT NULL,
                public_key_b64 TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS delegation (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                delegation_json TEXT NOT NULL,
                accepted_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS activity_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at TEXT NOT NULL,
                level TEXT NOT NULL,
                message TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // Load or generate identity.
        let identity = match load_identity(&pool).await? {
            Some(id) => id,
            None => {
                let id = generate_identity();
                persist_identity(&pool, &id).await?;
                id
            }
        };

        // Load delegation if present.
        let delegation = load_delegation(&pool).await?;
        let phase = if delegation.is_some() {
            AgentPhase::Ready
        } else {
            AgentPhase::AwaitingDelegation
        };

        let (tx, _) = broadcast::channel(64);

        Ok(Arc::new(AgentStore {
            pool,
            identity,
            inner: RwLock::new(Inner {
                delegation,
                phase,
                current_goal: None,
            }),
            activity_tx: tx,
        }))
    }

    pub fn public_key(&self) -> &PublicKey {
        self.identity.public_key()
    }

    /// The agent's identity wrapper — passes anywhere a `Signer` is required.
    pub fn identity(&self) -> &AgentIdentity {
        &self.identity
    }

    /// Compatibility accessor: return the underlying Keypair for code
    /// that hasn't migrated to `identity()` yet. Prefer `identity()`.
    pub fn keypair(&self) -> &Keypair {
        self.identity.as_keypair()
    }

    pub async fn phase(&self) -> AgentPhase {
        self.inner.read().await.phase
    }

    pub async fn delegation(&self) -> Option<Delegation> {
        self.inner.read().await.delegation.clone()
    }

    pub async fn current_goal(&self) -> Option<String> {
        self.inner.read().await.current_goal.clone()
    }

    pub async fn delegation_summary(&self) -> Option<DelegationSummary> {
        let g = self.inner.read().await;
        g.delegation.as_ref().map(|d| DelegationSummary {
            id_b64: URL_SAFE_NO_PAD.encode(d.id.0),
            scope_grants: d.scope.grant.to_vec(),
            not_after: d.not_after,
            constraint_count: d.constraints.len(),
        })
    }

    /// Accept a pasted delegation. Validates that:
    ///   - the JSON parses
    ///   - the delegation's agent pubkey matches *this* agent's
    ///   - the not_after is in the future
    ///   - the signature is valid
    ///
    /// Slice 10a: now uses `maat_agent::accept_delegation` for the
    /// pubkey/expiry/signature checks; this method retains the
    /// JSON-parse + persistence layers around it.
    pub async fn accept_delegation(&self, json: &str) -> Result<(), AcceptError> {
        let d: Delegation =
            serde_json::from_str(json).map_err(|e| AcceptError::Malformed(e.to_string()))?;

        // SDK does pubkey-match + expiry + signature in one call.
        maat_agent::accept_delegation(&d, self.identity.public_key())
            .map_err(|e| match e {
                maat_sdk_core::SdkError::Config(msg) if msg.contains("agent pubkey") => {
                    AcceptError::WrongAgent
                }
                maat_sdk_core::SdkError::Protocol(msg) if msg.contains("expired") => {
                    AcceptError::Expired
                }
                maat_sdk_core::SdkError::Crypto(msg) => AcceptError::SignatureInvalid(msg),
                other => AcceptError::Other(other.to_string()),
            })?;

        // Persist.
        let json = serde_json::to_string(&d).map_err(|e| AcceptError::Other(e.to_string()))?;
        let now_iso = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT OR REPLACE INTO delegation (id, delegation_json, accepted_at) VALUES (1, ?, ?)")
            .bind(&json)
            .bind(&now_iso)
            .execute(&self.pool)
            .await
            .map_err(|e| AcceptError::Other(e.to_string()))?;

        let mut g = self.inner.write().await;
        g.delegation = Some(d);
        g.phase = AgentPhase::Ready;
        drop(g);

        self.record_activity(ActivityLevel::Success, "delegation accepted; agent is Ready")
            .await;
        Ok(())
    }

    pub async fn clear_delegation(&self) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM delegation").execute(&self.pool).await?;
        let mut g = self.inner.write().await;
        g.delegation = None;
        g.phase = AgentPhase::AwaitingDelegation;
        g.current_goal = None;
        drop(g);
        self.record_activity(ActivityLevel::Info, "delegation cleared").await;
        Ok(())
    }

    pub async fn set_phase(&self, phase: AgentPhase) {
        self.inner.write().await.phase = phase;
    }

    pub async fn set_goal(&self, goal: String) -> Result<(), GoalError> {
        let mut g = self.inner.write().await;
        if g.phase != AgentPhase::Ready && g.phase != AgentPhase::Idle {
            return Err(GoalError::WrongPhase(g.phase));
        }
        g.current_goal = Some(goal);
        g.phase = AgentPhase::Active;
        Ok(())
    }

    pub async fn finish_goal(&self) {
        let mut g = self.inner.write().await;
        g.current_goal = None;
        g.phase = AgentPhase::Idle;
    }

    pub async fn record_activity(&self, level: ActivityLevel, message: impl Into<String>) {
        let entry = ActivityEntry {
            at: chrono::Utc::now(),
            level,
            message: message.into(),
        };
        // Persist (best effort).
        let _ = sqlx::query("INSERT INTO activity_log (at, level, message) VALUES (?, ?, ?)")
            .bind(entry.at.to_rfc3339())
            .bind(serde_json::to_string(&level).unwrap_or_default().trim_matches('"').to_string())
            .bind(&entry.message)
            .execute(&self.pool)
            .await;
        // Trim to cap.
        let _ = sqlx::query(
            "DELETE FROM activity_log WHERE id NOT IN (
                SELECT id FROM activity_log ORDER BY id DESC LIMIT ?
            )",
        )
        .bind(ACTIVITY_LOG_CAP)
        .execute(&self.pool)
        .await;
        // Broadcast.
        let _ = self.activity_tx.send(entry);
    }

    pub async fn recent_activity(&self, limit: i64) -> Vec<ActivityEntry> {
        let rows = sqlx::query(
            "SELECT at, level, message FROM activity_log ORDER BY id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        rows.into_iter()
            .filter_map(|r| {
                let at_str: String = r.try_get("at").ok()?;
                let level_str: String = r.try_get("level").ok()?;
                let message: String = r.try_get("message").ok()?;
                let at = chrono::DateTime::parse_from_rfc3339(&at_str).ok()?.with_timezone(&chrono::Utc);
                let level = match level_str.as_str() {
                    "success" => ActivityLevel::Success,
                    "warn" => ActivityLevel::Warn,
                    "error" => ActivityLevel::Error,
                    _ => ActivityLevel::Info,
                };
                Some(ActivityEntry { at, level, message })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error("delegation JSON is malformed: {0}")]
    Malformed(String),
    #[error("delegation's agent pubkey does not match this agent's keypair")]
    WrongAgent,
    #[error("delegation has already expired")]
    Expired,
    #[error("delegation signature is invalid: {0}")]
    SignatureInvalid(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum GoalError {
    #[error("agent is in phase {0:?}; cannot accept goal")]
    WrongPhase(AgentPhase),
}

// ─── Identity load / generate / persist ────────────────────────────────────

fn generate_identity() -> AgentIdentity {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    AgentIdentity::from_seed(seed)
}

async fn load_identity(pool: &SqlitePool) -> anyhow::Result<Option<AgentIdentity>> {
    let row = sqlx::query("SELECT seed_b64 FROM identity WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let seed_b64: String = row.try_get("seed_b64")?;
    let seed_vec = URL_SAFE_NO_PAD
        .decode(&seed_b64)
        .map_err(|e| anyhow::anyhow!("seed not valid base64url: {}", e))?;
    if seed_vec.len() != 32 {
        anyhow::bail!("persisted seed has wrong length");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_vec);
    Ok(Some(AgentIdentity::from_seed(seed)))
}

async fn persist_identity(pool: &SqlitePool, id: &AgentIdentity) -> anyhow::Result<()> {
    let seed_b64 = URL_SAFE_NO_PAD.encode(id.secret_seed());
    let pk_b64 = URL_SAFE_NO_PAD.encode(&id.public_key().key_data);
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO identity (id, seed_b64, public_key_b64, created_at) VALUES (1, ?, ?, ?)")
        .bind(seed_b64)
        .bind(pk_b64)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

async fn load_delegation(pool: &SqlitePool) -> anyhow::Result<Option<Delegation>> {
    let row = sqlx::query("SELECT delegation_json FROM delegation WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let json: String = row.try_get("delegation_json")?;
    Ok(Some(serde_json::from_str(&json)?))
}
