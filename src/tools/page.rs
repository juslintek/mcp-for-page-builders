use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn page_url(id: u64) -> String { format!("wp/v2/pages/{id}?context=edit") }

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(|s| s.to_string())
}

fn u64_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key)?.as_u64()
}

// ── CreatePage ────────────────────────────────────────────────────────────────

pub struct CreatePage;

#[async_trait]
impl Tool for CreatePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_page",
            description: "Create a new WordPress page with Elementor data. Returns the new page ID.",
            input_schema: json!({
                "type": "object",
                "required": ["title", "elementor_data"],
                "properties": {
                    "title": { "type": "string", "description": "Page title" },
                    "elementor_data": { "type": "string", "description": "Elementor JSON array string" },
                    "status": { "type": "string", "enum": ["publish","draft","private"], "default": "draft" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let title = str_arg(&args, "title").unwrap_or_default();
        let data = str_arg(&args, "elementor_data").unwrap_or_default();
        let status = str_arg(&args, "status").unwrap_or_else(|| "draft".into());

        // Validate JSON before sending
        serde_json::from_str::<Value>(&data)
            .map_err(|e| anyhow::anyhow!("elementor_data is not valid JSON: {e}"))?;

        let body = json!({
            "title": title,
            "status": status,
            "meta": { "_elementor_data": data, "_elementor_edit_mode": "builder" }
        });

        let result = wp.post("wp/v2/pages", &body).await?;
        let id = result["id"].as_u64().unwrap_or(0);
        wp.clear_elementor_cache().await?;

        Ok(ToolResult::text(format!("Created page ID {id}: {}", result["link"].as_str().unwrap_or(""))))
    }
}

// ── GetPage ───────────────────────────────────────────────────────────────────

pub struct GetPage;

#[async_trait]
impl Tool for GetPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_page",
            description: "Get a WordPress page by ID including its Elementor data.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer", "description": "Page ID" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let page = wp.get(&page_url(id)).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&page)?))
    }
}

// ── UpdatePage ────────────────────────────────────────────────────────────────

pub struct UpdatePage;

#[async_trait]
impl Tool for UpdatePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_page",
            description: "Update a page's title, status, and/or Elementor data. Clears CSS cache automatically.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "status": { "type": "string", "enum": ["publish","draft","private"] },
                    "elementor_data": { "type": "string", "description": "Full Elementor JSON array string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let mut body = json!({});

        if let Some(t) = str_arg(&args, "title") { body["title"] = json!(t); }
        if let Some(s) = str_arg(&args, "status") { body["status"] = json!(s); }
        if let Some(data) = str_arg(&args, "elementor_data") {
            serde_json::from_str::<Value>(&data)
                .map_err(|e| anyhow::anyhow!("elementor_data is not valid JSON: {e}"))?;
            body["meta"] = json!({ "_elementor_data": data });
        }

        wp.post(&format!("wp/v2/pages/{id}"), &body).await?;
        wp.clear_elementor_cache().await?;

        Ok(ToolResult::text(format!("Updated page {id} and cleared Elementor CSS cache.")))
    }
}

// ── DeletePage ────────────────────────────────────────────────────────────────

pub struct DeletePage;

#[async_trait]
impl Tool for DeletePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_page",
            description: "Delete a WordPress page.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer" },
                    "force": { "type": "boolean", "description": "Bypass trash", "default": false }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let path = if force { format!("wp/v2/pages/{id}?force=true") } else { format!("wp/v2/pages/{id}") };
        wp.delete(&path).await?;
        Ok(ToolResult::text(format!("Deleted page {id}.")))
    }
}

// ── GetPageBySlug ─────────────────────────────────────────────────────────────

pub struct GetPageBySlug;

#[async_trait]
impl Tool for GetPageBySlug {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_page_by_slug",
            description: "Look up a page ID from its URL slug.",
            input_schema: json!({
                "type": "object",
                "required": ["slug"],
                "properties": {
                    "slug": { "type": "string", "description": "URL slug (without slashes)" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let slug = str_arg(&args, "slug").ok_or_else(|| anyhow::anyhow!("slug required"))?;
        let result = wp.get(&format!("wp/v2/pages?slug={slug}&context=edit")).await?;
        let pages = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;
        if pages.is_empty() {
            return Ok(ToolResult::error(format!("No page found with slug '{slug}'")));
        }
        let id = pages[0]["id"].as_u64().unwrap_or(0);
        let title = pages[0]["title"]["rendered"].as_str().unwrap_or("");
        Ok(ToolResult::text(format!("Page ID {id}: {title}")))
    }
}

// ── ListPages ─────────────────────────────────────────────────────────────────

pub struct ListPages;

#[async_trait]
impl Tool for ListPages {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_pages",
            description: "List WordPress pages with their IDs, titles, slugs, and status.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "per_page": { "type": "integer", "default": 20, "maximum": 100 },
                    "page": { "type": "integer", "default": 1 },
                    "status": { "type": "string", "description": "Filter by status: publish, draft, any" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(20);
        let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
        let status = str_arg(&args, "status").unwrap_or_else(|| "any".into());

        let result = wp.get(&format!("wp/v2/pages?per_page={per_page}&page={page}&status={status}")).await?;
        let pages = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;

        let mut lines = vec![format!("Found {} pages:", pages.len())];
        for p in pages {
            let id = p["id"].as_u64().unwrap_or(0);
            let title = p["title"]["rendered"].as_str().unwrap_or("(no title)");
            let slug = p["slug"].as_str().unwrap_or("");
            let status = p["status"].as_str().unwrap_or("");
            lines.push(format!("  [{id}] {title} /{slug} ({status})"));
        }

        Ok(ToolResult::text(lines.join("\n")))
    }
}
