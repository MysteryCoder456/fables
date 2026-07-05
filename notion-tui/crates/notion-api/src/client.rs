use crate::error::ApiError;
use serde_json::Value;

pub const NOTION_VERSION: &str = "2025-09-03";

pub struct NotionClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
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
        }
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        self.request(reqwest::Method::GET, path, None).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.token)
            .header("Notion-Version", NOTION_VERSION);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let v: Value = resp.json().await.unwrap_or_default();
            return Err(ApiError::Api {
                status: status.as_u16(),
                code: v["code"].as_str().unwrap_or("unknown").to_string(),
                message: v["message"].as_str().unwrap_or("").to_string(),
            });
        }
        Ok(resp.json().await?)
    }
}
