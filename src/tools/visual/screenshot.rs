use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::{cdp_screenshot, unix_timestamp};

pub struct Screenshot;

#[async_trait]
impl Tool for Screenshot {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "screenshot",
            description: "Capture a full-page screenshot of any URL using headless Chrome. Saves to a PNG file.",
            input_schema: json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"},"output":{"type":"string","description":"Output PNG path. Defaults to /tmp/screenshot-{timestamp}.png"},"width":{"type":"integer","default":1440},"height":{"type":"integer","default":900}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let output = str_arg(&args, "output").unwrap_or_else(|| format!("/tmp/screenshot-{}.png", unix_timestamp()));
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);

        cdp_screenshot(&url, Path::new(&output), width, height).await?;
        Ok(ToolResult::text(format!("Screenshot saved to {output}")))
    }
}
