//! GET /mailer/v1/sends — list recent sends for audit.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;

use crate::repos;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SendView {
    pub id: String,
    pub recipients: Vec<String>,
    pub content_hash_hex: String,
    pub status: String,
    pub rejection_reason: Option<String>,
    pub maat_receipt_id_b64: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_sends(State(state): State<Arc<AppState>>) -> Response {
    match repos::list_recent(&state.pool, 100).await {
        Ok(rows) => {
            let views: Vec<SendView> = rows
                .into_iter()
                .map(|r| SendView {
                    id: r.id.to_string(),
                    recipients: serde_json::from_str(&r.recipients_json).unwrap_or_default(),
                    content_hash_hex: r.content_hash_hex,
                    status: r.status,
                    rejection_reason: r.rejection_reason,
                    maat_receipt_id_b64: URL_SAFE_NO_PAD.encode(r.maat_receipt_id),
                    created_at: r.created_at,
                })
                .collect();
            (StatusCode::OK, Json(views)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("list failed: {}", e) })),
        )
            .into_response(),
    }
}
