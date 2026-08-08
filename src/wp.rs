use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::util::config_dir;

#[derive(Clone, Serialize, Deserialize)]
pub struct SiteCredentials {
    pub url: String,
    pub user: String,
    pub app_password: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct SiteStore {
    pub sites: HashMap<String, SiteCredentials>,
    #[serde(default)]
    pub active: Option<String>,
}

impl SiteStore {
    fn path() -> PathBuf {
        config_dir().join("sites.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(config_dir())?;
        std::fs::write(Self::path(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn add_site(&mut self, creds: SiteCredentials) {
        let key = creds.url.clone();
        self.sites.insert(key.clone(), creds);
        if self.active.is_none() {
            self.active = Some(key);
        }
    }

    pub fn remove_site(&mut self, url: &str) {
        self.sites.remove(url);
        if self.active.as_deref() == Some(url) {
            self.active = self.sites.keys().next().cloned();
        }
    }

    pub fn list_sites(&self) -> Vec<&SiteCredentials> {
        self.sites.values().collect()
    }

    pub fn get_active(&self) -> Option<&SiteCredentials> {
        self.active.as_ref().and_then(|u| self.sites.get(u))
    }

    pub fn switch(&mut self, url: &str) -> Result<()> {
        if self.sites.contains_key(url) {
            self.active = Some(url.to_string());
            Ok(())
        } else {
            anyhow::bail!("Site not found: {url}")
        }
    }
}

pub type SharedStore = Arc<RwLock<SiteStore>>;

/// Notifications that tools can send to the main loop.
#[derive(Debug)]
pub enum ServerNotification {
    /// `WpClient` should be rebuilt from the current active site in the store.
    Reconfigure,
    /// Tool list has changed — send `notifications/tools/list_changed`.
    #[allow(dead_code)]
    ToolsChanged,
}

pub struct WpClient {
    http: reqwest::Client,
    base_url: String,
    auth: String,
    configured: bool,
    store: Option<SharedStore>,
    pub session: Option<Arc<crate::session::Session>>,
    notify_tx: Option<tokio::sync::mpsc::UnboundedSender<ServerNotification>>,
}

async fn parse_wp_response(resp: reqwest::Response, context_label: &str) -> Result<Value> {
    let status = resp.status();
    let text = resp
        .text()
        .await
        .with_context(|| format!("{context_label}: failed to read response body"))?;
    let trimmed = text.trim();

    if trimmed.is_empty() {
        if status.is_success() {
            return Ok(Value::Null);
        }
        anyhow::bail!("WP API {status}: empty response body");
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(json_val) => {
            if !status.is_success() {
                anyhow::bail!(
                    "WP API {status}: {}",
                    serde_json::to_string_pretty(&json_val).unwrap_or_else(|_| trimmed.to_string())
                );
            }
            Ok(json_val)
        }
        Err(json_err) => {
            if let (Some(start), Some(end)) = (trimmed.find(['{', '[']), trimmed.rfind(['}', ']']))
                && start < end
                && let Ok(json_val) = serde_json::from_str::<Value>(&trimmed[start..=end])
            {
                if !status.is_success() {
                    anyhow::bail!(
                        "WP API {status}: {}",
                        serde_json::to_string_pretty(&json_val).unwrap_or_else(|_| trimmed.to_string())
                    );
                }
                return Ok(json_val);
            }

            let snippet = if trimmed.len() > 300 {
                format!("{}...", &trimmed[..300])
            } else {
                trimmed.to_string()
            };

            if !status.is_success() {
                anyhow::bail!("WP API HTTP {status} (non-JSON response): {snippet}");
            }

            anyhow::bail!("{context_label}: error decoding response body ({json_err}): {snippet}");
        }
    }
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
            configured: true,
            store: None,
            session: None,
            notify_tx: None,
        }
    }

    #[must_use]
    pub fn with_store(mut self, store: SharedStore) -> Self {
        self.store = Some(store);
        self
    }

    #[must_use]
    pub fn with_session(mut self, s: Arc<crate::session::Session>) -> Self {
        self.session = Some(s);
        self
    }

    pub fn unconfigured() -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: String::new(),
            auth: String::new(),
            configured: false,
            store: None,
            session: None,
            notify_tx: None,
        }
    }

    pub fn from_creds(creds: &SiteCredentials) -> Self {
        Self::new(&creds.url, &creds.user, &creds.app_password)
    }

    pub const fn is_configured(&self) -> bool {
        self.configured
    }

    pub const fn store(&self) -> Option<&SharedStore> {
        self.store.as_ref()
    }

    #[must_use]
    pub fn with_notifier(mut self, tx: tokio::sync::mpsc::UnboundedSender<ServerNotification>) -> Self {
        self.notify_tx = Some(tx);
        self
    }

    /// Send a notification to the main loop.
    pub fn notify(&self, n: ServerNotification) {
        if let Some(tx) = &self.notify_tx {
            let _ = tx.send(n);
        }
    }

    /// Rebuild this client from the given credentials, preserving store/session/notifier.
    pub fn reconfigure(&mut self, creds: &SiteCredentials) {
        let fresh = Self::new(&creds.url, &creds.user, &creds.app_password);
        self.http = fresh.http;
        self.base_url = fresh.base_url;
        self.auth = fresh.auth;
        self.configured = fresh.configured;
    }

    pub fn require_configured(&self) -> Result<()> {
        if self.configured { return Ok(()); }
        anyhow::bail!(
            "WordPress not configured. Call the setup_wizard tool to connect:\n\
             • Option A: Provide WP_URL, username, and app password via setup_wizard\n\
             • Option B: Set WP_URL, WP_APP_USER, WP_APP_PASSWORD env vars in your MCP config\n\
             • Option C: Run `mcp-for-page-builders setup <wordpress-url>` from the command line"
        )
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/wp-json/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.require_configured()?;
        let resp = self.http.get(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send().await.context("WP GET request failed")?;
        parse_wp_response(resp, "WP GET parse failed").await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.require_configured()?;
        let resp = self.http.post(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send().await.context("WP POST request failed")?;
        parse_wp_response(resp, "WP POST parse failed").await
    }

    #[allow(dead_code)]
    pub async fn put(&self, path: &str, body: &Value) -> Result<Value> {
        self.require_configured()?;
        let resp = self.http.put(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send().await.context("WP PUT request failed")?;
        parse_wp_response(resp, "WP PUT parse failed").await
    }

    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.require_configured()?;
        let resp = self.http.delete(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send().await.context("WP DELETE request failed")?;
        parse_wp_response(resp, "WP DELETE parse failed").await
    }

    pub async fn post_multipart(&self, path: &str, form: reqwest::multipart::Form) -> Result<Value> {
        self.require_configured()?;
        let resp = self.http.post(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .multipart(form)
            .send().await.context("WP multipart POST failed")?;
        parse_wp_response(resp, "WP multipart parse failed").await
    }

    pub async fn request(&self, method: &str, path: &str, body: Option<&Value>, query: Option<&Value>) -> Result<Value> {
        self.require_configured()?;
        let url = self.url(path);
        let mut req = match method.to_uppercase().as_str() {
            "GET" => self.http.get(&url),
            "POST" => self.http.post(&url),
            "PUT" => self.http.put(&url),
            "DELETE" => self.http.delete(&url),
            "PATCH" => self.http.patch(&url),
            other => anyhow::bail!("Unsupported HTTP method: {other}"),
        };
        req = req.header(AUTHORIZATION, &self.auth);
        if let Some(q) = query
            && let Some(obj) = q.as_object()
        {
            let pairs: Vec<(String, String)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), String::from)))
                .collect();
            req = req.query(&pairs);
        }
        if let Some(b) = body {
            req = req.header(CONTENT_TYPE, "application/json").json(b);
        }
        let resp = req.send().await.context("WP request failed")?;
        parse_wp_response(resp, "WP response parse failed").await
    }

    /// Clear Elementor CSS cache. Ignores errors — endpoint may not exist on older versions.
    pub async fn clear_elementor_cache(&self) -> Result<()> {
        let _ = self.delete("elementor/v1/cache").await;
        Ok(())
    }
}
