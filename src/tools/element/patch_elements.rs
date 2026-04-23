use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::u64_arg;
use crate::elementor::{merge_settings, ElementorService};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct PatchElements;

#[async_trait]
impl Tool for PatchElements {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "patch_elements",
            description: "Patch settings on multiple Elementor elements in a single read→mutate→write cycle. More efficient than calling update_element repeatedly — avoids multiple round-trips for bulk changes.\n\nWorkflow: Use when you need to update several elements on the same page at once (e.g. fix mobile widths on all columns).",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "patches"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["element_id", "settings"],
                            "properties": {
                                "element_id": { "type": "string" },
                                "settings": { "type": "object" }
                            }
                        }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let patches = args["patches"].as_array().ok_or_else(|| anyhow::anyhow!("patches required"))?;

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;

        let total = patches.len();
        let mut matched = 0usize;

        for patch in patches {
            let eid = patch["element_id"].as_str().ok_or_else(|| anyhow::anyhow!("element_id required in patch"))?;
            let new_settings = patch["settings"].clone();
            if tree.mutate(eid, |el| merge_settings(&mut el.settings, &new_settings)) {
                matched += 1;
            }
        }

        let jid = wp.session.as_ref().map(|s| s.record("patch_elements", wp.base_url(), &format!("page:{page_id}")));
        svc.save_tree(page_id, &tree).await?;
        if let (Some(s), Some(id)) = (&wp.session, jid) { s.complete(&id); }

        Ok(ToolResult::text(format!("Patched {matched}/{total} elements on page {page_id}")))
    }
}
