//! The agent's autonomous loop.
//!
//! Activated when the agent enters the Active phase (delegation
//! present + goal accepted). Runs as a tokio task. Streams its
//! progress into the activity log, which the UI subscribes to via
//! SSE.
//!
//! Slice 10a: rebuilt on the SDK.
//! - `GatewayClient` from `maat_agent` replaces the local gateway client.
//! - `AnchorBuilder` from `maat_agent` replaces the inline `Anchor::builder` chain.
//! - `verify_action::<ValueClaim>(&request, claim)` replaces the
//!   `VerifyRequest`/`VerifyOutcome` flow. The `ValueClaim` itself is
//!   the action descriptor — its bytes ride into `receipt.action.value`
//!   via the protocol's `action_value` field (Maat 0.3.0).

use std::sync::Arc;
use std::time::Duration;

use maat::verify::ActionRequest;
use maat::ValueClaim;
use maat_agent::{AnchorBuilder, GatewayClient};
use maat_sdk_core::SdkError;
use uuid::Uuid;

use crate::catalog::{CheckoutError, StoreClient};
use crate::store::{ActivityLevel, AgentPhase, AgentStore};
use crate::strategy::{DeterministicStrategy, Goal, Strategy};

/// Configured by the runtime when the loop is launched.
pub struct LoopConfig {
    pub agent_store: Arc<AgentStore>,
    pub gateway: GatewayClient,
    pub store: StoreClient,
    pub customer_id: Uuid,
    pub goal: Goal,
}

pub async fn run(cfg: LoopConfig) {
    let strategy = DeterministicStrategy;
    let LoopConfig {
        agent_store,
        gateway,
        store,
        customer_id,
        goal,
    } = cfg;

    agent_store
        .record_activity(
            ActivityLevel::Info,
            format!(
                "loop started: budget {}¢, max-per-item {}¢{}",
                goal.max_total_cents,
                goal.max_per_item_cents,
                goal.mode
                    .as_deref()
                    .map(|m| format!(", mode={}", m))
                    .unwrap_or_default()
            ),
        )
        .await;

    let Some(delegation) = agent_store.delegation().await else {
        agent_store
            .record_activity(ActivityLevel::Error, "no delegation; aborting")
            .await;
        agent_store.set_phase(AgentPhase::AwaitingDelegation).await;
        return;
    };
    let chain = vec![delegation.clone()];

    // Fetch catalog.
    let catalog = match store.list_products().await {
        Ok(c) => {
            agent_store
                .record_activity(
                    ActivityLevel::Info,
                    format!("fetched catalog: {} items", c.len()),
                )
                .await;
            c
        }
        Err(e) => {
            agent_store
                .record_activity(
                    ActivityLevel::Error,
                    format!("catalog fetch failed: {}", e),
                )
                .await;
            agent_store.finish_goal().await;
            return;
        }
    };

    let mut spent: i64 = 0;
    let mut bought_ids: Vec<Uuid> = Vec::new();

    loop {
        // Strategy picks the next item.
        let chosen = match strategy.pick_next(&catalog, spent, &bought_ids, &goal) {
            Some(c) => c,
            None => {
                agent_store
                    .record_activity(
                        ActivityLevel::Info,
                        format!(
                            "no further items fit goal; total spent {}¢ across {} items",
                            spent,
                            bought_ids.len()
                        ),
                    )
                    .await;
                break;
            }
        };

        agent_store
            .record_activity(
                ActivityLevel::Info,
                format!(
                    "picked '{}' ({}¢), claiming {}¢",
                    chosen.product.name, chosen.product.price_cents, chosen.claim_amount_cents
                ),
            )
            .await;

        // Build the value claim — currency/decimals from the product.
        let claim = ValueClaim {
            currency: chosen.product.currency.clone(),
            amount: chosen.claim_amount_cents.max(0) as u64,
            decimals: 2,
        };

        // Build anchor via the SDK. Empty state for the demo — anchors
        // normally carry the agent's view of the world but for this
        // scenario the relevant world-state is the catalog itself.
        let anchor = match AnchorBuilder::new(delegation.id.clone())
            .max_staleness(Duration::from_secs(300))
            .build(agent_store.identity())
        {
            Ok(a) => a,
            Err(e) => {
                agent_store
                    .record_activity(
                        ActivityLevel::Error,
                        format!("anchor build failed: {}", e),
                    )
                    .await;
                break;
            }
        };

        // Build the action request. value_claim feeds MaxValue
        // enforcement at the gateway; the SDK's verify_action
        // additionally encodes the descriptor (here, the same claim)
        // into `action_value` for the receipt's action.value bytes.
        let request = ActionRequest {
            delegation: delegation.clone(),
            delegation_chain: chain.clone(),
            anchor,
            action_scope: "commerce:purchase:execute".into(),
            domain: None,
            value_claim: Some(claim.clone()),
            action_value: None, // GatewayClient fills from descriptor
        };

        // Verify with the gateway.
        let verified = match gateway.verify_action(&request, claim).await {
            Ok(v) => v,
            Err(SdkError::Rejected { reason, detail, .. }) => {
                let lvl = match reason.as_str() {
                    "constraint_violated" => ActivityLevel::Warn,
                    "revoked" | "expired" => ActivityLevel::Warn,
                    _ => ActivityLevel::Error,
                };
                agent_store
                    .record_activity(
                        lvl,
                        format!(
                            "gateway rejected '{}': {} ({})",
                            chosen.product.name, reason, detail
                        ),
                    )
                    .await;
                if reason == "revoked" || reason == "expired" {
                    break;
                }
                if detail.contains("cumulative cap") {
                    break;
                }
                bought_ids.push(chosen.product.id);
                continue;
            }
            Err(e) => {
                agent_store
                    .record_activity(ActivityLevel::Error, format!("gateway error: {}", e))
                    .await;
                break;
            }
        };

        agent_store
            .record_activity(
                ActivityLevel::Success,
                format!(
                    "gateway approved '{}' for {}¢",
                    chosen.product.name, chosen.product.price_cents
                ),
            )
            .await;

        // Forward receipt JSON bytes to the store as the X-Maat-Receipt header.
        let receipt_json = String::from_utf8_lossy(&verified.receipt_json).into_owned();
        match store
            .checkout(customer_id, chosen.product.id, &receipt_json)
            .await
        {
            Ok(ok) => {
                spent += chosen.product.price_cents;
                bought_ids.push(chosen.product.id);
                agent_store
                    .record_activity(
                        ActivityLevel::Success,
                        format!(
                            "store charged {}¢ via stripe ({}); running total {}¢",
                            ok.amount_cents,
                            ok.stripe_payment_intent_id.as_deref().unwrap_or("?"),
                            spent
                        ),
                    )
                    .await;
            }
            Err(CheckoutError::Rejected { status, detail }) => {
                agent_store
                    .record_activity(
                        ActivityLevel::Error,
                        format!(
                            "store rejected '{}' ({}): {}",
                            chosen.product.name, status, detail
                        ),
                    )
                    .await;
                bought_ids.push(chosen.product.id);
            }
            Err(CheckoutError::Transport(e)) => {
                agent_store
                    .record_activity(
                        ActivityLevel::Error,
                        format!("store transport error: {}", e),
                    )
                    .await;
                break;
            }
        }
    }

    agent_store
        .record_activity(
            ActivityLevel::Info,
            format!("loop complete; spent {}¢ across {} items", spent, bought_ids.len()),
        )
        .await;
    agent_store.finish_goal().await;
}
