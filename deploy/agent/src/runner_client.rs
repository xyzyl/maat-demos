use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RunnerClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct DeployBody<'a> {
    environment: &'a str,
    service: &'a str,
    commit_hash: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct DeployOk {
    pub deploy_id: uuid::Uuid,
    pub status: String,
    pub environment: String,
    pub service: String,
    pub commit_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("transport: {0}")] Transport(String),
    #[error("rejected ({status}): {detail}")] Rejected { status: u16, detail: String },
}

impl RunnerClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build().expect("reqwest"),
        }
    }

    pub async fn deploy(
        &self,
        receipt_json: &str,
        environment: &str,
        service: &str,
        commit_hash: &str,
    ) -> Result<DeployOk, DeployError> {
        let url = format!("{}/runner/v1/deploy", self.base_url.trim_end_matches('/'));
        let body = DeployBody { environment, service, commit_hash };
        let resp = self.http.post(&url)
            .header("X-Maat-Receipt", receipt_json)
            .json(&body).send().await
            .map_err(|e| DeployError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            serde_json::from_str(&text).map_err(|e| DeployError::Transport(e.to_string()))
        } else {
            let detail = serde_json::from_str::<serde_json::Value>(&text).ok()
                .and_then(|v| v.get("error").and_then(|s| s.as_str().map(String::from)))
                .unwrap_or(text);
            Err(DeployError::Rejected { status: status.as_u16(), detail })
        }
    }
}
