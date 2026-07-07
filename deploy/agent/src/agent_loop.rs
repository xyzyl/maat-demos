//! Deploy agent autonomous loop.
//!
//! Goal shape: { environment, service, commit_hash }. The agent builds
//! an anchor bound to (commit_hash + environment), constructs an
//! ActionRequest with TargetClaim, calls the gateway, forwards the
//! receipt to the runner. Hierarchical scope `ops:deploy:{env}` is
//! used so an `ops:deploy` delegation grants any environment while
//! `ops:deploy:staging` restricts to staging only.

use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maat::verify::ActionRequest;
use maat_agent::{AnchorBuilder, GatewayClient, StateBinding};
use maat_sdk_core::SdkError;

use crate::descriptor::TargetClaim;
use crate::runner_client::{DeployError, RunnerClient};
use crate::store::{ActivityLevel, AgentPhase, AgentStore};

pub struct LoopConfig {
    pub agent_store: Arc<AgentStore>,
    pub gateway: GatewayClient,
    pub runner: RunnerClient,
    pub environment: String,
    pub service: String,
    pub commit_hash: String,
}

pub async fn run(cfg: LoopConfig) {
    let LoopConfig { agent_store, gateway, runner, environment, service, commit_hash } = cfg;

    agent_store.record(ActivityLevel::Info, format!(
        "deploy goal: {}@{} → {}", service, commit_hash, environment
    )).await;

    let Some(delegation) = agent_store.delegation().await else {
        agent_store.record(ActivityLevel::Error, "no delegation; aborting").await;
        agent_store.set_phase(AgentPhase::AwaitingDelegation).await;
        return;
    };
    let chain = vec![delegation.clone()];

    let claim = TargetClaim {
        environment: environment.clone(),
        service: service.clone(),
        commit_hash: commit_hash.clone(),
    };

    // Anchor bound to commit hash and target environment — pins the
    // exact code+target combination cryptographically.
    let commit_bytes = hex::decode(&commit_hash).unwrap_or_else(|_| commit_hash.as_bytes().to_vec());
    let anchor = match AnchorBuilder::new(delegation.id.clone())
        .add_state(StateBinding::content_hash("commit_hash", commit_bytes))
        .add_state(StateBinding::content_hash("environment", environment.as_bytes().to_vec()))
        .max_staleness(Duration::from_secs(30))
        .build(agent_store.identity())
    {
        Ok(a) => a,
        Err(e) => {
            agent_store.record(ActivityLevel::Error, format!("anchor build: {}", e)).await;
            agent_store.set_phase(AgentPhase::Idle).await;
            return;
        }
    };

    // Hierarchical scope: ops:deploy:{env}
    let action_scope = format!("ops:deploy:{}", environment);

    let request = ActionRequest {
        delegation: delegation.clone(),
        delegation_chain: chain.clone(),
        anchor,
        action_scope,
        domain: None,
        value_claim: None,
        action_value: None, // gateway fills from descriptor
    };

    agent_store.record(ActivityLevel::Info, "requesting authorization from gateway").await;
    let verified = match gateway.verify_action(&request, claim.clone()).await {
        Ok(v) => v,
        Err(SdkError::Rejected { reason, detail, .. }) => {
            let lvl = match reason.as_str() {
                "constraint_violated" | "expired" | "revoked" | "scope_violated" => ActivityLevel::Warn,
                _ => ActivityLevel::Error,
            };
            agent_store.record(lvl, format!("gateway rejected: {} ({})", reason, detail)).await;
            agent_store.set_phase(AgentPhase::Idle).await;
            return;
        }
        Err(e) => {
            agent_store.record(ActivityLevel::Error, format!("gateway error: {}", e)).await;
            agent_store.set_phase(AgentPhase::Idle).await;
            return;
        }
    };

    agent_store.record(ActivityLevel::Success, format!(
        "gateway approved: receipt {}", URL_SAFE_NO_PAD.encode(verified.receipt.id.0)
    )).await;

    let receipt_str = String::from_utf8_lossy(&verified.receipt_json).into_owned();
    match runner.deploy(&receipt_str, &environment, &service, &commit_hash).await {
        Ok(ok) => {
            agent_store.record(ActivityLevel::Success, format!(
                "runner deployed {}@{} to {} (deploy_id={})",
                ok.service, ok.commit_hash, ok.environment, ok.deploy_id
            )).await;
        }
        Err(DeployError::Rejected { status, detail }) => {
            agent_store.record(ActivityLevel::Error, format!(
                "runner rejected ({}): {}", status, detail
            )).await;
        }
        Err(DeployError::Transport(e)) => {
            agent_store.record(ActivityLevel::Error, format!("runner transport: {}", e)).await;
        }
    }

    agent_store.set_phase(AgentPhase::Idle).await;
}
