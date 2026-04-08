use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::{cdp_screenshot, unix_timestamp, comparison_html};

pub struct VisualCompare;

#[async_trait]
impl Tool for VisualCompare {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_compare",
            description: "Screenshot two URLs side-by-side and generate an HTML comparison file.",
            input_schema: json!({"type":"object","required":["url_a","url_b"],"properties":{"url_a":{"type":"string"},"url_b":{"type":"string"},"label_a":{"type":"string","default":"A"},"label_b":{"type":"string","default":"B"},"output_dir":{"type":"string"},"width":{"type":"integer","default":1440},"height":{"type":"integer","default":900}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let label_a = str_arg(&args, "label_a").unwrap_or_else(|| "A".into());
        let label_b = str_arg(&args, "label_b").unwrap_or_else(|| "B".into());
        let dir = str_arg(&args, "output_dir").unwrap_or_else(|| "/tmp".into());
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);

        let t = unix_timestamp();
        let dir = Path::new(&dir);
        let img_a = dir.join(format!("compare-a-{t}.png"));
        let img_b = dir.join(format!("compare-b-{t}.png"));
        let html_out = dir.join(format!("compare-{t}.html"));

        cdp_screenshot(&url_a, &img_a, width, height).await?;
        cdp_screenshot(&url_b, &img_b, width, height).await?;

        let html = comparison_html(&label_a, img_a.file_name().unwrap().to_str().unwrap(), &label_b, img_b.file_name().unwrap().to_str().unwrap(), &url_a, &url_b);
        tokio::fs::write(&html_out, &html).await?;

        Ok(ToolResult::text(format!("Comparison saved to {}\nOpen in browser: open {}", html_out.display(), html_out.display())))
    }
}
