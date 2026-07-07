//! Demo autonomous agent.
//!
//! Runs as a small web service so that the operator can paste a
//! delegation into a UI rather than a CLI argument. See architecture
//! document Section 3.2.
//!
//! Slice 10a: the local `gateway_client` module is removed; agents
//! now use `maat_agent::GatewayClient` from the SDK.

pub mod agent_loop;
pub mod catalog;
pub mod config;
pub mod handlers;
pub mod runtime;
pub mod store;
pub mod strategy;

pub use config::Config;
pub use runtime::Runtime;
pub use store::AgentStore;

use std::sync::Arc;

use axum::{
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};

const INDEX_HTML: &str = include_str!("static/index.html");

pub fn build_router(rt: Arc<Runtime>) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/agent/v1/status", get(handlers::status))
        .route(
            "/agent/v1/delegation",
            post(handlers::accept_delegation).delete(handlers::clear_delegation),
        )
        .route("/agent/v1/goal", post(handlers::submit_goal))
        .route("/agent/v1/log", get(handlers::activity_stream))
        .route("/agent/v1/health", get(|| async { "ok" }))
        .with_state(rt)
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}
