pub mod agent_loop;
pub mod config;
pub mod descriptor;
pub mod handlers;
pub mod runner_client;
pub mod runtime;
pub mod store;

pub use config::Config;
pub use runtime::Runtime;
pub use store::AgentStore;

use std::sync::Arc;

use axum::{
    response::{Html, IntoResponse},
    routing::{get, post},
    Extension, Router,
};

const INDEX_HTML: &str = include_str!("static/index.html");

pub fn build_router(rt: Arc<Runtime>) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/agent/v1/status", get(handlers::status))
        .route("/agent/v1/delegation",
            post(handlers::accept_delegation).delete(handlers::clear_delegation))
        .route("/agent/v1/goal", post(handlers::submit_goal))
        .route("/agent/v1/log", get(handlers::activity_stream))
        .route("/agent/v1/health", get(|| async { "ok" }))
        .layer(Extension(rt))
}

async fn serve_index() -> impl IntoResponse { Html(INDEX_HTML) }
