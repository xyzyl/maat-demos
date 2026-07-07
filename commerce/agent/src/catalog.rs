//! Client for the store's catalog and checkout endpoints.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: Uuid,
    pub sku: String,
    pub name: String,
    pub description: String,
    pub price_cents: i64,
    pub currency: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckoutRequest {
    pub customer_id: Uuid,
    pub product_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckoutOk {
    pub transaction_id: Uuid,
    pub status: String,
    pub stripe_payment_intent_id: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
}

pub struct StoreClient {
    base: String,
    http: reqwest::Client,
}

impl StoreClient {
    pub fn new(base: impl Into<String>) -> Self {
        StoreClient {
            base: base.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn list_products(&self) -> anyhow::Result<Vec<Product>> {
        let url = format!("{}/store/v1/products", self.base.trim_end_matches('/'));
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("catalog fetch returned {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    pub async fn checkout(
        &self,
        customer_id: Uuid,
        product_id: Uuid,
        receipt_json: &str,
    ) -> Result<CheckoutOk, CheckoutError> {
        let url = format!("{}/store/v1/checkout", self.base.trim_end_matches('/'));
        let body = CheckoutRequest {
            customer_id,
            product_id,
        };

        let resp = self
            .http
            .post(&url)
            .header("X-Maat-Receipt", receipt_json)
            .json(&body)
            .send()
            .await
            .map_err(|e| CheckoutError::Transport(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| CheckoutError::Transport(e.to_string()))?;

        if status.is_success() {
            let ok: CheckoutOk = serde_json::from_str(&text)
                .map_err(|e| CheckoutError::Transport(format!("malformed success: {}", e)))?;
            Ok(ok)
        } else {
            let err_msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or(text.clone());
            Err(CheckoutError::Rejected {
                status: status.as_u16(),
                detail: err_msg,
            })
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckoutError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("checkout rejected ({status}): {detail}")]
    Rejected { status: u16, detail: String },
}
