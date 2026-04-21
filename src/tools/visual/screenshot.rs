use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

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
            description: "Capture a full-page screenshot of any URL. Returns the image inline (visible to AI) and saves to disk.\n\nWorkflow: Use for visual verification after making changes. Use pre_js to interact with the page before capture (e.g. click a menu button, dismiss a cookie banner).",
            input_schema: json!({"type":"object","required":["url"],"properties":{
                "url":{"type":"string"},
                "output":{"type":"string","description":"Output PNG path. Defaults to /tmp/screenshot-{timestamp}.png"},
                "width":{"type":"integer","default":1440},
                "height":{"type":"integer","default":900},
                "pre_js":{"type":"string","description":"JavaScript to execute before screenshot (e.g. click a button, expand a menu)"},
                "wait_ms":{"type":"integer","default":0,"description":"Milliseconds to wait after pre_js before capturing"}
            }}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let output = str_arg(&args, "output").unwrap_or_else(|| format!("/tmp/screenshot-{}.png", unix_timestamp()));
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);
        let pre_js = str_arg(&args, "pre_js");
        let wait_ms = u64_arg(&args, "wait_ms").unwrap_or(0);

        let (bytes, warning) = cdp_screenshot(&url, std::path::Path::new(&output), width, height, pre_js.as_deref(), wait_ms).await?;
        let mut msg = format!("Screenshot saved to {output}");
        if let Some(w) = warning { msg.push_str(&format!("\n⚠ {w}")); }
        Ok(ToolResult::text_and_image(msg, &bytes, "image/png"))
    }
}
