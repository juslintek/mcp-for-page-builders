use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::{SiteCredentials, WpClient};

pub struct ListSites;
pub struct ConnectSite;
pub struct DisconnectSite;
pub struct SwitchSite;

#[async_trait]
impl Tool for ListSites {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_sites",
            description: "List all stored WordPress site connections",
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let store = wp.store().ok_or_else(|| anyhow::anyhow!("Site store not available"))?;
        let s = store.read().await;
        let sites: Vec<Value> = s.list_sites().iter().map(|c| {
            json!({
                "url": c.url,
                "user": c.user,
                "active": s.active.as_deref() == Some(&c.url),
            })
        }).collect();
        Ok(ToolResult::text(serde_json::to_string_pretty(&json!({"sites": sites}))?))
    }
}

#[async_trait]
impl Tool for ConnectSite {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "connect_site",
            description: "Add a WordPress site connection",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "WordPress site URL"},
                    "user": {"type": "string", "description": "WordPress username"},
                    "app_password": {"type": "string", "description": "Application password"}
                },
                "required": ["url", "user", "app_password"]
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("url required"))?
            .trim_end_matches('/').to_string();
        let user = args["user"].as_str().ok_or_else(|| anyhow::anyhow!("user required"))?.to_string();
        let app_password = args["app_password"].as_str().ok_or_else(|| anyhow::anyhow!("app_password required"))?.to_string();

        let store = wp.store().ok_or_else(|| anyhow::anyhow!("Site store not available"))?;
        let mut s = store.write().await;
        s.add_site(SiteCredentials { url: url.clone(), user, app_password });
        s.save()?;
        Ok(ToolResult::text(format!("Connected to {url}. Use switch_site to make it active.")))
    }
}

#[async_trait]
impl Tool for DisconnectSite {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "disconnect_site",
            description: "Remove a stored WordPress site connection",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "WordPress site URL to remove"}
                },
                "required": ["url"]
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("url required"))?;
        let store = wp.store().ok_or_else(|| anyhow::anyhow!("Site store not available"))?;
        let mut s = store.write().await;
        s.remove_site(url);
        s.save()?;
        Ok(ToolResult::text(format!("Disconnected from {url}")))
    }
}

#[async_trait]
impl Tool for SwitchSite {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "switch_site",
            description: "Switch the active WordPress site",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "WordPress site URL to switch to"}
                },
                "required": ["url"]
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("url required"))?;
        let store = wp.store().ok_or_else(|| anyhow::anyhow!("Site store not available"))?;
        let mut s = store.write().await;
        s.switch(url)?;
        s.save()?;
        Ok(ToolResult::text(format!("Switched active site to {url}. Restart the server or reconnect for the change to take effect.")))
    }
}
