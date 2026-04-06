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
        let resp = self.http.get(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send().await
            .context("WP GET request failed")?;
        let status = resp.status();
        let body: Value = resp.json().await.context("WP GET parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&body)?);
        }
        Ok(body)
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self.http.post(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send().await
            .context("WP POST request failed")?;
        let status = resp.status();
        let result: Value = resp.json().await.context("WP POST parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&result)?);
        }
        Ok(result)
    }

    pub async fn delete(&self, path: &str) -> Result<Value> {
        let resp = self.http.delete(self.url(path))
            .header(AUTHORIZATION, &self.auth)
            .send().await
            .context("WP DELETE request failed")?;
        let status = resp.status();
        let body: Value = resp.json().await.context("WP DELETE parse failed")?;
        if !status.is_success() {
            anyhow::bail!("WP API {status}: {}", serde_json::to_string_pretty(&body)?);
        }
        Ok(body)
    }

    /// Clear Elementor CSS cache — call after every write operation.
    pub async fn clear_elementor_cache(&self) -> Result<()> {
        // Ignore errors — endpoint may not exist on older Elementor versions
        let _ = self.delete("elementor/v1/cache").await;
        Ok(())
    }

    /// Set Theme Builder display conditions for a template.
    ///
    /// Elementor Pro requires conditions in TWO places:
    /// 1. Post meta `_elementor_conditions` — a plain array like `["include/general"]`
    /// 2. WordPress option `elementor_pro_theme_builder_conditions` — maps type→id→conditions
    ///
    /// After setting both, the CSS cache must be cleared.
    pub async fn set_template_conditions(
        &self,
        template_id: u64,
        template_type: &str,
        conditions: &[String],
    ) -> Result<()> {
        // 1. Set post meta — send as plain array (WP REST serializes to PHP array)
        let cond_values: Vec<Value> = conditions.iter().map(|c| Value::String(c.clone())).collect();
        self.post(
            &format!("wp/v2/elementor_library/{template_id}"),
            &serde_json::json!({ "meta": { "_elementor_conditions": cond_values } }),
        ).await?;

        // 2. Update the global conditions option
        // Read current value
        let current = self.get("elementor-mcp/v1/option/elementor_pro_theme_builder_conditions").await
            .unwrap_or(serde_json::json!({}));
        let mut map = match current.as_object() {
            Some(m) => m.clone(),
            None => serde_json::Map::new(),
        };

        // Add this template under its type
        let type_entry = map.entry(template_type.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = type_entry.as_object_mut() {
            obj.insert(
                template_id.to_string(),
                Value::Array(cond_values.clone()),
            );
        }

        // Write back
        self.post(
            "elementor-mcp/v1/option/elementor_pro_theme_builder_conditions",
            &Value::Object(map),
        ).await.ok(); // best-effort — endpoint may not exist yet

        self.clear_elementor_cache().await?;
        Ok(())
    }
}
