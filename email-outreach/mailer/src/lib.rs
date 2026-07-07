pub mod config;
pub mod descriptor;
pub mod handlers;
pub mod repos;
pub mod state;

pub use config::Config;
pub use state::AppState;

use std::sync::Arc;

use axum::{
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};

const INDEX_HTML: &str = include_str!("static/index.html");

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/mailer/v1/send", post(handlers::send::handler))
        .route("/mailer/v1/sends", get(handlers::reads::list_sends))
        .route("/mailer/v1/health", get(|| async { "ok" }))
        .with_state(state)
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}
