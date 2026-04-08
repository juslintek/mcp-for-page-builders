use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::{cdp_screenshot, unix_timestamp};

pub struct ScreenshotPage;

#[async_trait]
impl Tool for ScreenshotPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "screenshot_page",
            description: "Screenshot a WordPress page by ID. Optionally append query params.",
            input_schema: json!({"type":"object","required":["page_id"],"properties":{"page_id":{"type":"integer"},"query":{"type":"string"},"output":{"type":"string"},"width":{"type":"integer","default":1440},"height":{"type":"integer","default":900}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let query = str_arg(&args, "query").unwrap_or_default();
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);

        let page = wp.get(&format!("wp/v2/pages/{page_id}")).await?;
        let link = page["link"].as_str().ok_or_else(|| anyhow::anyhow!("Page has no link"))?;
        let url = if query.is_empty() { link.to_string() } else { format!("{link}?{query}") };

        let output = str_arg(&args, "output").unwrap_or_else(|| format!("/tmp/page-{page_id}-{}.png", unix_timestamp()));
        cdp_screenshot(&url, Path::new(&output), width, height).await?;
        Ok(ToolResult::text(format!("Screenshot of page {page_id} saved to {output}\nURL: {url}")))
    }
}
