use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

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
        let per_page = args.get("per_page").and_then(serde_json::Value::as_u64).unwrap_or(20);
        let page = args.get("page").and_then(serde_json::Value::as_u64).unwrap_or(1);
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
