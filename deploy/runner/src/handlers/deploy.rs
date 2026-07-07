//! POST /runner/v1/deploy — the trust boundary.
//!
//! 0.1.1: validation failures (signature, freshness, replay) now also
//! produce audit rows. Best-effort parse of the receipt provides the
//! receipt id + delegation id for the rejection row even when the
//! validator refuses to honor the receipt's contents.

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

use crate::descriptor::TargetClaim;
use crate::repos::{self, NewDeploy};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct DeployRequest {
    pub environment: String,
    pub service: String,
    pub commit_hash: String,
}

#[derive(Debug, Serialize)]
pub struct DeployResponse {
    pub deploy_id: Uuid,
    pub status: String,
    pub environment: String,
    pub service: String,
    pub commit_hash: String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<DeployRequest>,
) -> Response {
    let receipt_header = match headers.get("X-Maat-Receipt") {
        Some(v) => v,
        None => return bad_request("missing X-Maat-Receipt header"),
    };
    let receipt_str = match receipt_header.to_str() {
        Ok(s) => s,
        Err(_) => return bad_request("X-Maat-Receipt header not UTF-8"),
    };

    // ── Best-effort metadata extraction BEFORE validation. ──
    // If the receipt parses, we get its id + delegation_id even if
    // validation will reject it. If parse fails (truly malformed),
    // we fall back to zero IDs so the audit row still exists.
    let (peek_receipt_id, peek_delegation_id): ([u8; 32], [u8; 32]) =
        match serde_json::from_str::<Receipt>(receipt_str) {
            Ok(r) => (r.id.0, r.delegation_id.0),
            Err(_) => ([0u8; 32], [0u8; 32]),
        };

    let validated = match state.validator.validate(receipt_str.as_bytes()).await {
        Ok(v) => v,
        Err(e) => {
            // Record the validation failure for audit.
            let reason = format_validation_error(&e);
            let _ = repos::record(&state.pool, NewDeploy {
                environment: &req.environment,
                service: &req.service,
                commit_hash: &req.commit_hash,
                status: "rejected",
                rejection_reason: Some(&reason),
                maat_receipt_id: &peek_receipt_id,
                maat_delegation_id: &peek_delegation_id,
            }).await;
            return validation_error_to_response(e);
        }
    };
    let receipt = validated.receipt;
    let claim: TargetClaim = validated.descriptor;

    // ── Layered enforcement: claim ↔ request equality. ──
    if claim.environment != req.environment {
        return rejected(&state, &receipt, &claim, &req,
            &format!("environment mismatch: claim '{}' vs request '{}'", claim.environment, req.environment)).await;
    }
    if claim.service != req.service {
        return rejected(&state, &receipt, &claim, &req,
            &format!("service mismatch: claim '{}' vs request '{}'", claim.service, req.service)).await;
    }
    if claim.commit_hash != req.commit_hash {
        return rejected(&state, &receipt, &claim, &req,
            &format!("commit_hash mismatch: claim '{}' vs request '{}'", claim.commit_hash, req.commit_hash)).await;
    }

    if !state.config.allowed_environments.iter().any(|e| e == &req.environment) {
        return rejected(&state, &receipt, &claim, &req,
            &format!("environment '{}' not allow-listed", req.environment)).await;
    }
    if !state.config.allowed_services.iter().any(|s| s == &req.service) {
        return rejected(&state, &receipt, &claim, &req,
            &format!("service '{}' not allow-listed", req.service)).await;
    }

    tracing::info!(
        env = %req.environment, svc = %req.service, commit = %req.commit_hash,
        "deploy: stub-executing"
    );

    let rec = match repos::record(&state.pool, NewDeploy {
        environment: &req.environment,
        service: &req.service,
        commit_hash: &req.commit_hash,
        status: "deployed",
        rejection_reason: None,
        maat_receipt_id: &receipt.id.0,
        maat_delegation_id: &receipt.delegation_id.0,
    }).await {
        Ok(r) => r,
        Err(e) => return server_error(&format!("record deploy: {}", e)),
    };

    (StatusCode::OK, Json(DeployResponse {
        deploy_id: rec.id,
        status: rec.status,
        environment: rec.environment,
        service: rec.service,
        commit_hash: rec.commit_hash,
    })).into_response()
}

async fn rejected(
    state: &AppState,
    receipt: &maat::Receipt,
    claim: &TargetClaim,
    req: &DeployRequest,
    reason: &str,
) -> Response {
    let _ = repos::record(&state.pool, NewDeploy {
        environment: &req.environment,
        service: &req.service,
        commit_hash: &claim.commit_hash,
        status: "rejected",
        rejection_reason: Some(reason),
        maat_receipt_id: &receipt.id.0,
        maat_delegation_id: &receipt.delegation_id.0,
    }).await;
    bad_request(reason)
}

/// Format a `ValidationError` as a stable, human-readable string for
/// the audit `rejection_reason` column. The taxonomy mirrors the
/// gateway's failure-receipt codes so audit logs stay consistent
/// between gateway-side and resource-side rejections.
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
        ValidationError::Malformed(m) => bad_request(&format!("receipt malformed: {}", m)),
        ValidationError::UntrustedExecutor => unauthorized("untrusted executor"),
        ValidationError::InvalidSignature(_) => unauthorized("invalid signature"),
        ValidationError::MalformedDescriptor(m) => bad_request(&format!("descriptor: {}", m)),
        ValidationError::FutureTimestamp { .. } => bad_request("executed_at in future"),
        ValidationError::TooOld { age_seconds, max_age_seconds } =>
            bad_request(&format!("too old: {}s, limit {}s", age_seconds, max_age_seconds)),
        ValidationError::Replay => conflict("already used"),
        ValidationError::Storage(m) => server_error(&format!("storage: {}", m)),
    }
}

fn bad_request(m: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": m}))).into_response()
}
fn unauthorized(m: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": m}))).into_response()
}
fn conflict(m: &str) -> Response {
    (StatusCode::CONFLICT, Json(serde_json::json!({"error": m}))).into_response()
}
fn server_error(m: &str) -> Response {
    tracing::error!("runner server error: {}", m);
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "internal"}))).into_response()
}
