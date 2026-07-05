use crate::error::ApiError;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

pub const NOTION_VERSION: &str = "2025-09-03";

const MAX_ATTEMPTS: u32 = 5;

pub struct NotionClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
    min_interval: Duration,
    backoff_base: Duration,
    next_allowed: Mutex<Instant>,
}

impl NotionClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, "https://api.notion.com")
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            min_interval: Duration::from_millis(334),
            backoff_base: Duration::from_millis(250),
            next_allowed: Mutex::new(Instant::now()),
        }
    }

    /// Test hook: shrink pacing/backoff so retry tests run fast.
    pub fn set_timing(&mut self, min_interval: Duration, backoff_base: Duration) {
        self.min_interval = min_interval;
        self.backoff_base = backoff_base;
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        self.request(reqwest::Method::GET, path, None).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }

    pub async fn patch_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::PATCH, path, Some(body)).await
    }

    pub async fn delete_json(&self, path: &str) -> Result<Value, ApiError> {
        self.request(reqwest::Method::DELETE, path, None).await
    }

    async fn pace(&self) {
        let mut next = self.next_allowed.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = Instant::now().max(*next) + self.min_interval;
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        for attempt in 0..MAX_ATTEMPTS {
            self.pace().await;
            let mut req = self
                .http
                .request(method.clone(), format!("{}{}", self.base_url, path))
                .bearer_auth(&self.token)
                .header("Notion-Version", NOTION_VERSION);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt + 1 == MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                    continue;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(self.backoff_base * 2u32.pow(attempt));
                tokio::time::sleep(wait).await;
                continue;
            }
            if status.is_server_error() {
                tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                continue;
            }
            if !status.is_success() {
                let v: Value = resp.json().await.unwrap_or_default();
                return Err(ApiError::Api {
                    status: status.as_u16(),
                    code: v["code"].as_str().unwrap_or("unknown").to_string(),
                    message: v["message"].as_str().unwrap_or("").to_string(),
                });
            }
            return Ok(resp.json().await?);
        }
        Err(ApiError::RetriesExhausted(path.to_string()))
    }
}
