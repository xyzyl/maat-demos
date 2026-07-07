//! HTTP surface that the agent's UI talks to.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::agent_loop::{self, LoopConfig};
use crate::catalog::StoreClient;
use maat_agent::GatewayClient;
use crate::runtime::Runtime;
use crate::store::{AcceptError, ActivityEntry, AgentPhase, DelegationSummary};
use crate::strategy::Goal;

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub phase: AgentPhase,
    pub public_key_b64: String,
    pub delegation: Option<DelegationSummary>,
    pub current_goal: Option<String>,
}

pub async fn status(State(rt): State<Arc<Runtime>>) -> Json<StatusView> {
    let phase = rt.agent.phase().await;
    let pk_b64 = URL_SAFE_NO_PAD.encode(&rt.agent.public_key().key_data);
    let delegation = rt.agent.delegation_summary().await;
    let current_goal = rt.agent.current_goal().await;

    Json(StatusView {
        phase,
        public_key_b64: pk_b64,
        delegation,
        current_goal,
    })
}

#[derive(Debug, Deserialize)]
pub struct AcceptDelegationReq {
    pub delegation_json: String,
}

pub async fn accept_delegation(
    State(rt): State<Arc<Runtime>>,
    Json(req): Json<AcceptDelegationReq>,
) -> Response {
    match rt.agent.accept_delegation(&req.delegation_json).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => {
            let code = match e {
                AcceptError::Malformed(_) | AcceptError::WrongAgent | AcceptError::Expired => {
                    StatusCode::BAD_REQUEST
                }
                AcceptError::SignatureInvalid(_) => StatusCode::UNAUTHORIZED,
                AcceptError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (code, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

pub async fn clear_delegation(State(rt): State<Arc<Runtime>>) -> Response {
    match rt.agent.clear_delegation().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct GoalReq {
    pub max_total_cents: i64,
    pub max_per_item_cents: i64,
    pub mode: Option<String>,
}

pub async fn submit_goal(
    State(rt): State<Arc<Runtime>>,
    Json(req): Json<GoalReq>,
) -> Response {
    if req.max_total_cents <= 0 || req.max_per_item_cents <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "limits must be positive"})),
        )
            .into_response();
    }
    let goal = Goal {
        max_total_cents: req.max_total_cents,
        max_per_item_cents: req.max_per_item_cents,
        mode: req.mode,
    };

    if let Err(e) = rt
        .agent
        .set_goal(format!(
            "spend up to {}¢ ({}¢ per item)",
            goal.max_total_cents, goal.max_per_item_cents
        ))
        .await
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    // Spawn the loop.
    let agent = rt.agent.clone();
    let gateway = GatewayClient::new(rt.config.gateway_url.clone(), rt.api_key.clone());
    let store = StoreClient::new(rt.config.store_url.clone());
    let customer_id = rt.commerce_customer_id;
    tokio::spawn(async move {
        agent_loop::run(LoopConfig {
            agent_store: agent,
            gateway,
            store,
            customer_id,
            goal,
        })
        .await;
    });

    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

/// SSE stream of activity entries.
///
/// Sends a "snapshot" of recent entries on connect, then streams new
/// entries as they're recorded.
pub async fn activity_stream(
    State(rt): State<Arc<Runtime>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let recent = rt.agent.recent_activity(50).await;
    let receiver = rt.agent.activity_tx.subscribe();

    let snapshot = stream::iter(recent.into_iter().map(|e| Ok(activity_event(&e))));
    let live = BroadcastStream::new(receiver).filter_map(|r| r.ok().map(|e| Ok(activity_event(&e))));

    let combined = snapshot.chain(live);
    Sse::new(combined).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn activity_event(e: &ActivityEntry) -> Event {
    Event::default()
        .event("activity")
        .data(serde_json::to_string(e).unwrap_or_default())
}
