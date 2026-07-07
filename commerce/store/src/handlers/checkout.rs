//! `POST /store/v1/checkout` — the trust boundary.
//!
//! Slice 10a: the 13-step fail-closed flow collapses onto the SDK's
//! `ReceiptValidator`. Steps 2-6 (header parse, JSON parse, executor
//! trust, signature, value-claim decode), step 8 (freshness), and
//! step 9 (replay) are now a single `state.validator.validate(...).await`
//! call. Steps 7 (amount-match) and 10-13 (Stripe + record) remain
//! resource-specific code.
//!
//! BEHAVIOR NOTE: in the pre-SDK flow, an amount mismatch rejected
//! the request BEFORE the receipt was marked consumed. With the SDK,
//! validate() consumes the receipt as part of its atomic check
//! pipeline, so an amount mismatch happens AFTER consumption — the
//! receipt is "burned." This is the correct security default: a
//! receipt cryptographically commits to a specific amount; if it
//! doesn't match the actual cart, it's the wrong receipt and must
//! never be redeemed for any other transaction. The audit trail
//! still captures the rejection via `record_transaction` with
//! `rejection_reason`.

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maat::Receipt;
use maat_resource::ValidationError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::repos::{self, NewTransaction};
use crate::state::AppState;
use crate::stripe_client::StripeError;

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub customer_id: Uuid,
    pub product_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    pub transaction_id: Uuid,
    pub status: String,
    pub stripe_payment_intent_id: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CheckoutRequest>,
) -> Response {
    // Step 2: receipt header.
    let receipt_header = match headers.get("X-Maat-Receipt") {
        Some(v) => v,
        None => return bad_request("missing X-Maat-Receipt header"),
    };
    let receipt_str = match receipt_header.to_str() {
        Ok(s) => s,
        Err(_) => return bad_request("X-Maat-Receipt header is not valid UTF-8"),
    };

    // Steps 3, 4, 6, 8, 9: SDK validator does parse + signature +
    // executor trust + descriptor decode + freshness + replay in one call.
    let validated = match state.validator.validate(receipt_str.as_bytes()).await {
        Ok(v) => v,
        Err(e) => return validation_error_to_response(e),
    };
    let receipt = validated.receipt;
    let claim = validated.descriptor;

    // Step 5: look up the product.
    let product = match repos::get_product(&state.pool, req.product_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return not_found("product not found"),
        Err(e) => return server_error(&format!("product lookup failed: {}", e)),
    };

    // Step 7: amount-match. THE LAYERED ENFORCEMENT CHECK.
    // The receipt says "agent committed to spending X". The cart says
    // "this product costs Y". If they differ, the receipt was issued
    // for something that isn't this purchase, and we refuse to honor
    // it regardless of how cryptographically valid it is.
    if claim.currency != product.currency {
        return rejected_with_record(
            &state,
            &req,
            &product,
            &receipt,
            &format!(
                "currency mismatch: claim '{}' vs product '{}'",
                claim.currency, product.currency
            ),
        )
        .await;
    }
    if claim.amount as i64 != product.price_cents {
        return rejected_with_record(
            &state,
            &req,
            &product,
            &receipt,
            &format!(
                "amount mismatch: claim {} vs product {}",
                claim.amount, product.price_cents
            ),
        )
        .await;
    }

    // Step 10: customer must exist.
    let customer = match repos::get_customer(&state.pool, req.customer_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("customer not found"),
        Err(e) => return server_error(&format!("customer lookup failed: {}", e)),
    };

    // Step 11: Stripe charge.
    let receipt_id_b64 = URL_SAFE_NO_PAD.encode(receipt.id.0);
    let delegation_id_b64 = URL_SAFE_NO_PAD.encode(receipt.delegation_id.0);
    let metadata = vec![
        ("maat_receipt_id", receipt_id_b64.clone()),
        ("maat_delegation_id", delegation_id_b64),
        ("product_sku", product.sku.clone()),
    ];

    let stripe_outcome = state
        .stripe
        .charge_customer(
            &customer.stripe_customer_id,
            product.price_cents,
            &product.currency,
            &metadata,
        )
        .await;

    match stripe_outcome {
        // Step 12: success path.
        Ok(charge) => {
            let tx = NewTransaction {
                customer_id: customer.id,
                product_id: product.id,
                amount_cents: product.price_cents,
                currency: &product.currency,
                status: "succeeded",
                stripe_payment_intent_id: Some(&charge.payment_intent_id),
                stripe_error: None,
                maat_receipt_id: &receipt.id.0,
                maat_delegation_id: &receipt.delegation_id.0,
                rejection_reason: None,
            };
            match repos::record_transaction(&state.pool, tx).await {
                Ok(rec) => {
                    let resp = CheckoutResponse {
                        transaction_id: rec.id,
                        status: rec.status,
                        stripe_payment_intent_id: rec.stripe_payment_intent_id,
                        amount_cents: rec.amount_cents,
                        currency: rec.currency,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
                Err(e) => server_error(&format!("transaction record failed: {}", e)),
            }
        }
        // Step 13: failure path.
        Err(StripeError::Declined(msg)) | Err(StripeError::Call(msg)) => {
            let tx = NewTransaction {
                customer_id: customer.id,
                product_id: product.id,
                amount_cents: product.price_cents,
                currency: &product.currency,
                status: "failed",
                stripe_payment_intent_id: None,
                stripe_error: Some(&msg),
                maat_receipt_id: &receipt.id.0,
                maat_delegation_id: &receipt.delegation_id.0,
                rejection_reason: None,
            };
            let _ = repos::record_transaction(&state.pool, tx).await;
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": "stripe charge failed", "detail": msg })),
            )
                .into_response()
        }
        Err(other) => server_error(&other.to_string()),
    }
}

/// Map SDK validation errors to HTTP responses.
fn validation_error_to_response(e: ValidationError) -> Response {
    match e {
        ValidationError::Malformed(msg) => bad_request(&format!("receipt malformed: {}", msg)),
        ValidationError::UntrustedExecutor => {
            tracing::warn!("checkout: receipt signed by an executor we don't trust");
            unauthorized("receipt was not issued by a trusted executor")
        }
        ValidationError::InvalidSignature(_) => unauthorized("receipt signature invalid"),
        ValidationError::MalformedDescriptor(msg) => {
            bad_request(&format!("value claim malformed: {}", msg))
        }
        ValidationError::FutureTimestamp { .. } => {
            bad_request("receipt executed_at is in the future")
        }
        ValidationError::TooOld {
            age_seconds,
            max_age_seconds,
        } => bad_request(&format!(
            "receipt too old: {}s, limit {}s",
            age_seconds, max_age_seconds
        )),
        ValidationError::Replay => conflict("receipt already used"),
        ValidationError::Storage(msg) => server_error(&format!("replay-check storage error: {}", msg)),
    }
}

/// Helper for amount-mismatch / currency-mismatch rejections — record
/// the rejection in transactions for audit, then return 400. Note that
/// the receipt has already been marked consumed by the validator at
/// this point (see module-level BEHAVIOR NOTE).
async fn rejected_with_record(
    state: &AppState,
    req: &CheckoutRequest,
    product: &repos::Product,
    receipt: &Receipt,
    reason: &str,
) -> Response {
    let tx = NewTransaction {
        customer_id: req.customer_id,
        product_id: req.product_id,
        amount_cents: product.price_cents,
        currency: &product.currency,
        status: "rejected",
        stripe_payment_intent_id: None,
        stripe_error: None,
        maat_receipt_id: &receipt.id.0,
        maat_delegation_id: &receipt.delegation_id.0,
        rejection_reason: Some(reason),
    };
    let _ = repos::record_transaction(&state.pool, tx).await;
    bad_request(reason)
}

// ─── Response helpers ──────────────────────────────────────────────────────

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn conflict(msg: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

fn server_error(msg: &str) -> Response {
    tracing::error!("checkout server error: {}", msg);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": "internal error" })),
    )
        .into_response()
}
