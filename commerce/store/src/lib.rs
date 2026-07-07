//! Demo merchant — a real(ish) e-commerce service that honors Maat
//! receipts.
//!
//! Slice 10a: rebuilt on the SDK. The local `executor_key` module is
//! gone; `maat_resource::fetch_executor_key` does that job. The
//! `handlers::checkout` flow uses `maat_resource::ReceiptValidator`
//! for the protocol-level checks, leaving only resource-specific
//! logic (amount-match, Stripe, transaction record) in handler code.

pub mod config;
pub mod handlers;
pub mod repos;
pub mod state;
pub mod stripe_client;

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
        .route("/store/v1/products", get(handlers::reads::list_products))
        .route(
            "/store/v1/products/:id",
            get(handlers::reads::get_product),
        )
        .route("/store/v1/checkout", post(handlers::checkout::handler))
        .route(
            "/store/v1/transactions",
            get(handlers::reads::list_transactions),
        )
        .route("/store/v1/health", get(|| async { "ok" }))
        .with_state(state)
}

async fn serve_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}
