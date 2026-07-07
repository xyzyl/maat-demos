//! Thin HTTP client to the mailer's send endpoint.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct MailerClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct SendBody<'a> {
    recipients: &'a [String],
    body: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct SendOk {
    pub send_id: uuid::Uuid,
    pub status: String,
    pub recipients_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("rejected ({status}): {detail}")]
    Rejected { status: u16, detail: String },
}

impl MailerClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn send(
        &self,
        receipt_json: &str,
        recipients: &[String],
        body: &str,
    ) -> Result<SendOk, SendError> {
        let url = format!("{}/mailer/v1/send", self.base_url.trim_end_matches('/'));
        let body = SendBody { recipients, body };
        let resp = self
            .http
            .post(&url)
            .header("X-Maat-Receipt", receipt_json)
            .json(&body)
            .send()
            .await
            .map_err(|e| SendError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            serde_json::from_str(&text).map_err(|e| SendError::Transport(e.to_string()))
        } else {
            let detail = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("error").and_then(|s| s.as_str().map(String::from)))
                .unwrap_or(text);
            Err(SendError::Rejected {
                status: status.as_u16(),
                detail,
            })
        }
    }
}
