use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{bool_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct CreatePreviewLink;
pub struct PublishDocument;
pub struct ListSiteParts;
pub struct ManageSiteParts;

#[async_trait]
impl Tool for CreatePreviewLink {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_preview_link",
            description: "Creates a signed, time-limited anonymous preview URL for a draft/autosave Elementor document without publishing it, plus an edit_url for the user.",
            input_schema: json!({
                "type": "object",
                "required": ["post_id"],
                "properties": {
                    "post_id": { "type": "integer", "description": "Post ID to snapshot and preview." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let post_id =
            u64_arg(&args, "post_id").ok_or_else(|| anyhow::anyhow!("post_id required"))?;
        let resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/preview-link",
                &json!({ "post_id": post_id }),
            )
            .await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for PublishDocument {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "publish_document",
            description: "Transitions an Elementor document (page, post, theme template, popup) to 'publish' and promotes any pending autosave to the live document.",
            input_schema: json!({
                "type": "object",
                "required": ["post_id"],
                "properties": {
                    "post_id": { "type": "integer", "description": "WordPress post ID of the Elementor document to publish." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let post_id =
            u64_arg(&args, "post_id").ok_or_else(|| anyhow::anyhow!("post_id required"))?;
        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/publish-document",
                &json!({ "post_id": post_id }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Post(post_id), &mut resp)
            .await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ListSiteParts {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_site_parts",
            description: "Lists Theme Builder site parts (header, footer, single, archive, 404, etc.) with conditions, publish state, and whether each part is active on the front end.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "active": { "type": "boolean", "description": "When true, return only site parts currently rendered on the front end." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let active_only = bool_arg(&args, "active").unwrap_or(false);
        let resp = wp
            .get(&format!(
                "mcp-for-page-builders/v1/v4/site-parts?active={active_only}"
            ))
            .await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ManageSiteParts {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "manage_site_parts",
            description: "Bulk create, update conditions, or delete Elementor Theme Builder site parts (header, footer, single, archive, error-404, etc.). Created parts remain drafts until published.",
            input_schema: json!({
                "type": "object",
                "required": ["operations"],
                "properties": {
                    "operations": {
                        "type": "array",
                        "description": "Bulk operations (1-50).",
                        "items": {
                            "type": "object",
                            "required": ["action"],
                            "properties": {
                                "action": { "type": "string", "enum": ["create", "update", "delete"] },
                                "type": {
                                    "type": "string",
                                    "enum": [
                                        "section", "header", "footer", "single", "single-post",
                                        "single-page", "archive", "search-results", "error-404",
                                        "code_snippet", "loop-item"
                                    ]
                                },
                                "title": { "type": "string" },
                                "conditions": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "post_id": { "type": "integer" }
                            }
                        }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let ops = args
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("operations array required"))?;
        if ops.is_empty() || ops.len() > 50 {
            bail!("operations must contain between 1 and 50 items");
        }
        for (idx, op) in ops.iter().enumerate() {
            let action = op
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("operations[{idx}].action required"))?;
            match action {
                "create" => {
                    if op.get("type").and_then(Value::as_str).is_none() {
                        bail!("operations[{idx}] create requires type");
                    }
                }
                "update" => {
                    if op.get("post_id").and_then(Value::as_u64).is_none() {
                        bail!("operations[{idx}] update requires post_id");
                    }
                }
                "delete" => {
                    if op.get("post_id").and_then(Value::as_u64).is_none() {
                        bail!("operations[{idx}] delete requires post_id");
                    }
                }
                other => bail!("operations[{idx}] unknown action '{other}'"),
            }
        }
        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/site-parts",
                &json!({ "operations": ops }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}
