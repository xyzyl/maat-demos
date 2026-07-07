//! Outreach agent autonomous loop.
//!
//! Goal shape: { recipients: [..], body: "..." }. For each recipient
//! the agent: builds an anchor (content-hash-bound), constructs an
//! ActionRequest with RecipientClaim and a domain hint, calls the
//! gateway, forwards the receipt to the mailer.

use std::sync::Arc;
use std::time::Duration;

use maat::verify::ActionRequest;
use maat_agent::{AnchorBuilder, GatewayClient, StateBinding};
use maat_sdk_core::SdkError;

use crate::descriptor::{hex_digest, RecipientClaim};
use crate::mailer_client::{MailerClient, SendError};
use crate::store::{ActivityLevel, AgentPhase, AgentStore};

pub struct LoopConfig {
    pub agent_store: Arc<AgentStore>,
    pub gateway: GatewayClient,
    pub mailer: MailerClient,
    pub recipients: Vec<String>,
    pub body: String,
}

pub async fn run(cfg: LoopConfig) {
    let LoopConfig { agent_store, gateway, mailer, recipients, body } = cfg;

    agent_store.record(ActivityLevel::Info, format!(
        "starting outreach: {} recipients, body {} bytes",
        recipients.len(), body.len()
    )).await;

    let Some(delegation) = agent_store.delegation().await else {
        agent_store.record(ActivityLevel::Error, "no delegation; aborting").await;
        agent_store.set_phase(AgentPhase::AwaitingDelegation).await;
        return;
    };
    let chain = vec![delegation.clone()];
    let content_hash_hex = hex_digest(body.as_bytes());

    let mut sent = 0usize;
    let mut rejected = 0usize;

    for recipient in &recipients {
        // Determine domain from recipient.
        let domain = match recipient.rsplit_once('@') {
            Some((_, d)) => d.to_lowercase(),
            None => {
                agent_store.record(ActivityLevel::Warn,
                    format!("skipping malformed recipient '{}'", recipient)).await;
                continue;
            }
        };

        // Build the claim — single-recipient per send for clarity. (Multi
        // could be done with one claim; we keep it 1:1 so the audit trail
        // is granular per recipient.)
        let claim = RecipientClaim {
            recipients: vec![recipient.clone()],
            content_hash_hex: content_hash_hex.clone(),
        };

        // Build anchor with state binding to the body content hash —
        // cryptographically pins the message version.
        let anchor = match AnchorBuilder::new(delegation.id.clone())
            .add_state(StateBinding::content_hash(
                "outreach_body",
                hex::decode(&content_hash_hex).unwrap_or_default(),
            ))
            .max_staleness(Duration::from_secs(60))
            .build(agent_store.identity())
        {
            Ok(a) => a,
            Err(e) => {
                agent_store.record(ActivityLevel::Error, format!("anchor build: {}", e)).await;
                break;
            }
        };

        let request = ActionRequest {
            delegation: delegation.clone(),
            delegation_chain: chain.clone(),
            anchor,
            action_scope: "messaging:email:send".into(),
            domain: Some(domain.clone()),
            value_claim: None,
            action_value: None, // gateway fills from descriptor
        };

        let verified = match gateway.verify_action(&request, claim.clone()).await {
            Ok(v) => v,
            Err(SdkError::Rejected { reason, detail, .. }) => {
                let lvl = match reason.as_str() {
                    "constraint_violated" | "expired" | "revoked" => ActivityLevel::Warn,
                    _ => ActivityLevel::Error,
                };
                agent_store.record(lvl, format!(
                    "gateway rejected '{}': {} ({})", recipient, reason, detail
                )).await;
                rejected += 1;
                // Rate-limit rejection: stop the loop — further sends will fail until the window slides.
                if detail.contains("max_rate") || reason == "revoked" || reason == "expired" {
                    break;
                }
                continue;
            }
            Err(e) => {
                agent_store.record(ActivityLevel::Error, format!("gateway error: {}", e)).await;
                break;
            }
        };

        let receipt_str = String::from_utf8_lossy(&verified.receipt_json).into_owned();
        match mailer.send(&receipt_str, std::slice::from_ref(recipient), &body).await {
            Ok(ok) => {
                sent += 1;
                agent_store.record(ActivityLevel::Success,
                    format!("mailer sent to '{}' (send_id={})", recipient, ok.send_id)).await;
            }
            Err(SendError::Rejected { status, detail }) => {
                rejected += 1;
                agent_store.record(ActivityLevel::Error,
                    format!("mailer rejected '{}' ({}): {}", recipient, status, detail)).await;
            }
            Err(SendError::Transport(e)) => {
                agent_store.record(ActivityLevel::Error,
                    format!("mailer transport error: {}", e)).await;
                break;
            }
        }

        // Tiny breather between sends — not strictly necessary, but
        // gives MaxRate a fair chance to spread out invocations.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    agent_store.record(ActivityLevel::Info, format!(
        "outreach complete: {} sent, {} rejected (of {} attempted)",
        sent, rejected, recipients.len()
    )).await;
    agent_store.set_phase(AgentPhase::Idle).await;
}
