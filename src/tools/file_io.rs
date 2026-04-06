use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

// ── DownloadPage ──────────────────────────────────────────────────────────────

pub struct DownloadPage;

#[async_trait]
impl Tool for DownloadPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "download_page",
            description: "Download a page's Elementor data to a local JSON file.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "path"],
                "properties": {
                    "id": { "type": "integer", "description": "Page ID" },
                    "path": { "type": "string", "description": "Local file path to save to" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let path = str_arg(&args, "path").ok_or_else(|| anyhow::anyhow!("path required"))?;

        let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await?;
        let data = page.get("meta")
            .and_then(|m| m.get("_elementor_data"))
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data found on page {id}"))?;

        // Pretty-print the elementor data for readability
        let content = match data.as_str() {
            Some(s) => {
                let parsed: Value = serde_json::from_str(s)?;
                serde_json::to_string_pretty(&parsed)?
            }
            None => serde_json::to_string_pretty(data)?,
        };

        tokio::fs::write(&path, &content).await?;
        Ok(ToolResult::text(format!("Saved Elementor data for page {id} to {path} ({} bytes)", content.len())))
    }
}

// ── UploadPage ────────────────────────────────────────────────────────────────

pub struct UploadPage;

#[async_trait]
impl Tool for UploadPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "upload_page",
            description: "Update a page's Elementor data from a local JSON file. Clears CSS cache automatically.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "path"],
                "properties": {
                    "id": { "type": "integer", "description": "Page ID" },
                    "path": { "type": "string", "description": "Local JSON file path" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let path = str_arg(&args, "path").ok_or_else(|| anyhow::anyhow!("path required"))?;

        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;

        // Validate JSON
        let parsed: Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("File is not valid JSON: {e}"))?;

        // Re-serialize compact (Elementor expects compact JSON in meta)
        let compact = serde_json::to_string(&parsed)?;

        let body = json!({
            "meta": { "_elementor_data": compact }
        });

        wp.post(&format!("wp/v2/pages/{id}"), &body).await?;
        ElementorService::new(wp).clear_cache().await?;

        Ok(ToolResult::text(format!("Updated page {id} from {path} and cleared CSS cache.")))
    }
}

// ── BackupPage ────────────────────────────────────────────────────────────────

pub struct BackupPage;

#[async_trait]
impl Tool for BackupPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "backup_page",
            description: "Backup a page's current Elementor data to a timestamped file before making changes.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer", "description": "Page ID" },
                    "dir": { "type": "string", "description": "Backup directory (default: /tmp)" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let dir = str_arg(&args, "dir").unwrap_or_else(|| "/tmp".into());

        let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await?;
        let data = page.get("meta")
            .and_then(|m| m.get("_elementor_data"))
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data found on page {id}"))?;

        let content = match data.as_str() {
            Some(s) => {
                let parsed: Value = serde_json::from_str(s)?;
                serde_json::to_string_pretty(&parsed)?
            }
            None => serde_json::to_string_pretty(data)?,
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let filename = format!("elementor-backup-{id}-{timestamp}.json");
        let full_path = Path::new(&dir).join(&filename);

        tokio::fs::write(&full_path, &content).await?;
        Ok(ToolResult::text(format!("Backed up page {id} to {}", full_path.display())))
    }
}
