//! Read-side handlers — products and transaction log.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use uuid::Uuid;

use crate::repos;
use crate::state::AppState;

pub async fn list_products(State(state): State<Arc<AppState>>) -> Response {
    match repos::list_products(&state.pool).await {
        Ok(rows) => (StatusCode::OK, Json(rows)).into_response(),
        Err(e) => internal(&e.to_string()),
    }
}

pub async fn get_product(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Response {
    match repos::get_product(&state.pool, id).await {
        Ok(Some(p)) => (StatusCode::OK, Json(p)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not found" })),
        )
            .into_response(),
        Err(e) => internal(&e.to_string()),
    }
}

#[derive(Debug, Serialize)]
pub struct TransactionView {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub product_id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub stripe_payment_intent_id: Option<String>,
    pub stripe_error: Option<String>,
    pub maat_receipt_id_b64: String,
    pub maat_delegation_id_b64: String,
    pub rejection_reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_transactions(State(state): State<Arc<AppState>>) -> Response {
    match repos::list_transactions(&state.pool, 200).await {
        Ok(rows) => {
            let view: Vec<TransactionView> = rows
                .into_iter()
                .map(|t| TransactionView {
                    id: t.id,
                    customer_id: t.customer_id,
                    product_id: t.product_id,
                    amount_cents: t.amount_cents,
                    currency: t.currency,
                    status: t.status,
                    stripe_payment_intent_id: t.stripe_payment_intent_id,
                    stripe_error: t.stripe_error,
                    maat_receipt_id_b64: URL_SAFE_NO_PAD.encode(&t.maat_receipt_id),
                    maat_delegation_id_b64: URL_SAFE_NO_PAD.encode(&t.maat_delegation_id),
                    rejection_reason: t.rejection_reason,
                    created_at: t.created_at,
                })
                .collect();
            (StatusCode::OK, Json(view)).into_response()
        }
        Err(e) => internal(&e.to_string()),
    }
}

fn internal(msg: &str) -> Response {
    tracing::error!("store read error: {}", msg);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}
