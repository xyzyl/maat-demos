//! HTTP endpoints for the outreach agent's control plane.
//!
//!   GET    /agent/v1/status
//!   POST   /agent/v1/delegation     — accept pasted delegation JSON
//!   DELETE /agent/v1/delegation     — clear current delegation
//!   POST   /agent/v1/goal           — { recipients: [..], body: ".." }
//!   GET    /agent/v1/log            — SSE stream of recent activity

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Extension},
    http::StatusCode,
    response::{sse, IntoResponse, Response, Sse},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::stream::Stream;
use maat_agent::GatewayClient;
use serde::{Deserialize, Serialize};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use crate::agent_loop::{run as run_loop, LoopConfig};
use crate::mailer_client::MailerClient;
use crate::runtime::Runtime;
use crate::store::{ActivityEntry, AgentPhase};

pub mod sse_log {
    pub use super::activity_stream;
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub phase: AgentPhase,
    pub public_key_b64: String,
    pub delegation: Option<crate::store::DelegationSummary>,
    pub recent_activity: Vec<ActivityEntry>,
}

pub async fn status(Extension(rt): Extension<Arc<Runtime>>) -> impl IntoResponse {
    let phase = rt.agent.phase().await;
    let pk_b64 = URL_SAFE_NO_PAD.encode(&rt.agent.public_key().key_data);
    let delegation = rt.agent.delegation_summary().await;
    let recent_activity = rt.agent.recent_activity(50).await;
    Json(StatusResponse {
        phase,
        public_key_b64: pk_b64,
        delegation,
        recent_activity,
    })
}

#[derive(Debug, Deserialize)]
pub struct AcceptDelegationBody {
    pub delegation_json: String,
}

pub async fn accept_delegation(
    Extension(rt): Extension<Arc<Runtime>>,
    Json(body): Json<AcceptDelegationBody>,
) -> Response {
    match rt.agent.accept_delegation(&body.delegation_json).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn clear_delegation(Extension(rt): Extension<Arc<Runtime>>) -> Response {
    match rt.agent.clear_delegation().await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct GoalBody {
    pub recipients: Vec<String>,
    pub body: String,
}

pub async fn submit_goal(
    Extension(rt): Extension<Arc<Runtime>>,
    Json(goal): Json<GoalBody>,
) -> Response {
    if goal.recipients.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "recipients list is empty" })),
        )
            .into_response();
    }
    if goal.body.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "body is empty" })),
        )
            .into_response();
    }
    let phase = rt.agent.phase().await;
    if !matches!(phase, AgentPhase::Ready | AgentPhase::Idle) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("wrong phase: {:?}", phase) })),
        )
            .into_response();
    }
    rt.agent.set_phase(AgentPhase::Active).await;
    let agent_store = rt.agent.clone();
    let gateway = GatewayClient::new(rt.config.gateway_url.clone(), rt.config.api_key.clone());
    let mailer = MailerClient::new(rt.config.mailer_url.clone());

    tokio::spawn(async move {
        run_loop(LoopConfig {
            agent_store,
            gateway,
            mailer,
            recipients: goal.recipients,
            body: goal.body,
        })
        .await;
    });

    (StatusCode::ACCEPTED, Json(serde_json::json!({ "ok": true }))).into_response()
}

pub async fn activity_stream(
    Extension(rt): Extension<Arc<Runtime>>,
) -> Sse<impl Stream<Item = Result<sse::Event, std::convert::Infallible>>> {
    let rx = rt.agent.activity_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|m| match m {
        Ok(entry) => {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Some(Ok(sse::Event::default().data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(sse::KeepAlive::new().interval(Duration::from_secs(15)))
}
