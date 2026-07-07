//! POST /mailer/v1/send — the trust boundary.
//!
//! 0.1.1: validation failures now produce audit rows (best-effort
//! receipt metadata extraction before validate).

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use maat::Receipt;
use maat_resource::ValidationError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::descriptor::{hex_digest, RecipientClaim};
use crate::repos::{self, NewSend};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub recipients: Vec<String>,
    pub body: String,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub send_id: Uuid,
    pub status: String,
    pub recipients_count: usize,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<SendRequest>,
) -> Response {
    let receipt_header = match headers.get("X-Maat-Receipt") {
        Some(v) => v,
        None => return bad_request("missing X-Maat-Receipt header"),
    };
    let receipt_str = match receipt_header.to_str() {
        Ok(s) => s,
        Err(_) => return bad_request("X-Maat-Receipt header not UTF-8"),
    };

    // Best-effort metadata extraction before validation.
    let (peek_receipt_id, peek_delegation_id): ([u8; 32], [u8; 32]) =
        match serde_json::from_str::<Receipt>(receipt_str) {
            Ok(r) => (r.id.0, r.delegation_id.0),
            Err(_) => ([0u8; 32], [0u8; 32]),
        };

    let validated = match state.validator.validate(receipt_str.as_bytes()).await {
        Ok(v) => v,
        Err(e) => {
            let reason = format_validation_error(&e);
            let rcpts_json = serde_json::to_string(&req.recipients).unwrap_or_default();
            let body_hash = hex_digest(req.body.as_bytes());
            let _ = repos::record_send(&state.pool, NewSend {
                recipients_json: &rcpts_json,
                content_hash_hex: &body_hash,
                status: "rejected",
                rejection_reason: Some(&reason),
                maat_receipt_id: &peek_receipt_id,
                maat_delegation_id: &peek_delegation_id,
            }).await;
            return validation_error_to_response(e);
        }
    };
    let receipt = validated.receipt;
    let claim: RecipientClaim = validated.descriptor;

    // Layered enforcement.
    let mut claim_rcpts: Vec<&String> = claim.recipients.iter().collect();
    let mut req_rcpts: Vec<&String> = req.recipients.iter().collect();
    claim_rcpts.sort();
    req_rcpts.sort();
    if claim_rcpts != req_rcpts {
        return rejected(&state, &receipt, &claim, "recipient list does not match committed claim").await;
    }

    let actual_hash = hex_digest(req.body.as_bytes());
    if actual_hash != claim.content_hash_hex {
        return rejected(&state, &receipt, &claim,
            &format!("body hash mismatch (committed {} vs actual {})",
                     claim.content_hash_hex, actual_hash)).await;
    }

    for r in &req.recipients {
        let domain = match r.rsplit_once('@') {
            Some((_, d)) => d.to_lowercase(),
            None => return rejected(&state, &receipt, &claim,
                &format!("recipient '{}' is not a valid email address", r)).await,
        };
        if !state.config.allowed_domains.iter().any(|ad| ad.to_lowercase() == domain) {
            return rejected(&state, &receipt, &claim,
                &format!("recipient domain '{}' not in mailer allow-list", domain)).await;
        }
    }

    tracing::info!(
        recipients = req.recipients.len(),
        hash = %claim.content_hash_hex,
        "outreach: stub-send"
    );

    let rcpts_json = serde_json::to_string(&claim.recipients).unwrap_or_default();
    let rec = match repos::record_send(&state.pool, NewSend {
        recipients_json: &rcpts_json,
        content_hash_hex: &claim.content_hash_hex,
        status: "sent",
        rejection_reason: None,
        maat_receipt_id: &receipt.id.0,
        maat_delegation_id: &receipt.delegation_id.0,
    }).await {
        Ok(r) => r,
        Err(e) => return server_error(&format!("record_send failed: {}", e)),
    };

    (StatusCode::OK, Json(SendResponse {
        send_id: rec.id,
        status: rec.status,
        recipients_count: req.recipients.len(),
    })).into_response()
}

async fn rejected(
    state: &AppState,
    receipt: &maat::Receipt,
    claim: &RecipientClaim,
    reason: &str,
) -> Response {
    let rcpts_json = serde_json::to_string(&claim.recipients).unwrap_or_default();
    let _ = repos::record_send(&state.pool, NewSend {
        recipients_json: &rcpts_json,
        content_hash_hex: &claim.content_hash_hex,
        status: "rejected",
        rejection_reason: Some(reason),
        maat_receipt_id: &receipt.id.0,
        maat_delegation_id: &receipt.delegation_id.0,
    }).await;
    bad_request(reason)
}

fn format_validation_error(e: &ValidationError) -> String {
    match e {
        ValidationError::Malformed(m) => format!("receipt_malformed: {}", m),
        ValidationError::UntrustedExecutor => "untrusted_executor".into(),
        ValidationError::InvalidSignature(m) => format!("invalid_signature: {}", m),
        ValidationError::MalformedDescriptor(m) => format!("descriptor_malformed: {}", m),
        ValidationError::FutureTimestamp { .. } => "future_timestamp".into(),
        ValidationError::TooOld { age_seconds, max_age_seconds } =>
            format!("too_old: age={}s limit={}s", age_seconds, max_age_seconds),
        ValidationError::Replay => "replay".into(),
        ValidationError::Storage(m) => format!("storage: {}", m),
    }
}

fn validation_error_to_response(e: ValidationError) -> Response {
    match e {
        ValidationError::Malformed(msg) => bad_request(&format!("receipt malformed: {}", msg)),
        ValidationError::UntrustedExecutor => unauthorized("untrusted executor"),
        ValidationError::InvalidSignature(_) => unauthorized("invalid signature"),
        ValidationError::MalformedDescriptor(msg) => bad_request(&format!("descriptor malformed: {}", msg)),
        ValidationError::FutureTimestamp { .. } => bad_request("receipt executed_at in future"),
        ValidationError::TooOld { age_seconds, max_age_seconds } => bad_request(&format!(
            "receipt too old: {}s, limit {}s", age_seconds, max_age_seconds)),
        ValidationError::Replay => conflict("receipt already used"),
        ValidationError::Storage(msg) => server_error(&format!("replay storage error: {}", msg)),
    }
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}
fn unauthorized(msg: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": msg }))).into_response()
}
fn conflict(msg: &str) -> Response {
    (StatusCode::CONFLICT, Json(serde_json::json!({ "error": msg }))).into_response()
}
fn server_error(msg: &str) -> Response {
    tracing::error!("mailer server error: {}", msg);
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "internal error" }))).into_response()
}
