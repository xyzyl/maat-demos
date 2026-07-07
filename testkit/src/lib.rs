//! Shared test infrastructure for Maat demos.
//!
//! This crate does NOT spawn gateways or databases — that's the
//! operator's responsibility. It provides:
//!
//! - Helpers to construct test delegations programmatically (so demo
//!   integration tests don't have to invoke the dashboard UI).
//! - A thin HTTP client around `reqwest` for hitting demo resources
//!   with receipt headers.
//! - A waiting helper for the human-confirm flow's HTTP 202 path.
//!
//! Demo tests gate themselves on `MAAT_TESTKIT_GATEWAY_URL` and
//! `MAAT_TESTKIT_API_KEY` env vars. They run when those are set
//! against a working gateway + dashboard + KMS stack.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maat::{Delegation, Keypair, PublicKey};
use maat_agent::{DelegationBuilder, ScopeBuilder};
use maat_sdk_core::ConstraintSet;

/// Test fixture: env-driven configuration for running demo integration tests.
pub struct TestHarness {
    pub gateway_url: String,
    pub api_key: String,
    pub http: reqwest::Client,
}

impl TestHarness {
    /// Construct from environment. Returns Err if required vars are absent.
    pub fn from_env() -> anyhow::Result<Self> {
        let gateway_url = std::env::var("MAAT_TESTKIT_GATEWAY_URL")
            .map_err(|_| anyhow::anyhow!("MAAT_TESTKIT_GATEWAY_URL not set"))?;
        let api_key = std::env::var("MAAT_TESTKIT_API_KEY")
            .map_err(|_| anyhow::anyhow!("MAAT_TESTKIT_API_KEY not set"))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(TestHarness {
            gateway_url,
            api_key,
            http,
        })
    }

    /// Skip a test gracefully if env not set.
    pub fn try_from_env() -> Option<Self> {
        Self::from_env().ok()
    }
}

// ─── Delegation construction helpers ───────────────────────────────────────

/// Issue a delegation programmatically. Bypasses the dashboard for
/// integration-test ergonomics.
pub fn test_delegation(
    principal: &Keypair,
    agent: &PublicKey,
    scope_atoms: &[&str],
    constraints: ConstraintSet,
    valid_for: Duration,
) -> Result<Delegation, maat_sdk_core::SdkError> {
    let scope = ScopeBuilder::new()
        .grant(scope_atoms.iter().map(|s| s.to_string()))
        .build();
    DelegationBuilder::new(agent.clone())
        .scope(scope)
        .constraints(constraints)
        .valid_for(valid_for)
        .build(principal)
}

// ─── Resource-side HTTP helpers ────────────────────────────────────────────

/// POST a JSON body to a demo resource endpoint with an X-Maat-Receipt header.
pub async fn post_with_receipt<B: serde::Serialize>(
    http: &reqwest::Client,
    url: &str,
    receipt_json: &[u8],
    body: &B,
) -> anyhow::Result<reqwest::Response> {
    Ok(http
        .post(url)
        .header("X-Maat-Receipt", String::from_utf8_lossy(receipt_json).to_string())
        .json(body)
        .send()
        .await?)
}

// ─── Pending-confirm await helper ──────────────────────────────────────────

/// When a gateway returns HTTP 202 with a pending_id, this helper polls
/// for the verify request to succeed on retry. Intended for tests that
/// trigger RequireHumanConfirm and then need to approve out-of-band
/// before retrying.
pub async fn poll_until_approved(
    approve_fn: impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if approve_fn().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

// ─── Misc ──────────────────────────────────────────────────────────────────

pub fn b64_id(id: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(id)
}
