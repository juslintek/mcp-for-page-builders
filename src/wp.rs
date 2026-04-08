use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

pub struct WpClient {
    http: reqwest::Client,
    base_url: String,
    auth: String,
}

impl WpClient {
    pub fn new(base_url: &str, user: &str, app_password: &str) -> Self {
        let creds = format!("{user}:{app_password}");
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(&creds));
        let accept_invalid = std::env::var("WP_TLS_INSECURE").is_ok();
        Self {
            http: reqwest::Client::builder()
                .danger_accept_invalid_certs(accept_invalid)
                .build()
                .expect("Failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            auth,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/wp-json/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send()
            .await
            .context("WP GET request failed")?;
        let status = resp.status();
        let body: Value = resp.json().await.context("WP GET parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&body)?);
        }
        Ok(body)
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .http
            .post(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .context("WP POST request failed")?;
        let status = resp.status();
        let result: Value = resp.json().await.context("WP POST parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&result)?);
        }
        Ok(result)
    }

    pub async fn delete(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .delete(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send()
            .await
            .context("WP DELETE request failed")?;
        let status = resp.status();
        let body: Value = resp.json().await.context("WP DELETE parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&body)?);
        }
        Ok(body)
    }

    /// Clear Elementor CSS cache. Ignores errors — endpoint may not exist on older versions.
    pub async fn clear_elementor_cache(&self) -> Result<()> {
        let _ = self.delete("elementor/v1/cache").await;
        Ok(())
    }
}
