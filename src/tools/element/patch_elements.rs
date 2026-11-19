use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::u64_arg;
use crate::elementor::{ElementorService, merge_settings};
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct PatchElements;

#[async_trait]
impl Tool for PatchElements {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "patch_elements",
            description: "Patch settings on multiple Elementor elements in a single read→mutate→write cycle. More efficient than calling update_element repeatedly — avoids multiple round-trips for bulk changes.\n\nWorkflow: Use when you need to update several elements on the same page at once (e.g. fix mobile widths on all columns).",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "expected_revision", "patches"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "expected_revision": { "type": "string", "description": "Revision returned by get_element_tree" },
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
        let page_id =
            u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let patches = args["patches"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("patches required"))?;

        let expected_revision =
            crate::args::str_arg(&args, "expected_revision").ok_or_else(|| {
                anyhow::anyhow!("expected_revision required; read the page tree first")
            })?;
        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        ElementorService::require_revision(&tree, &expected_revision)?;

        let total = patches.len();
        let mut ids = Vec::with_capacity(total);
        for patch in patches {
            let eid = patch["element_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("element_id required in patch"))?;
            if !patch["settings"].is_object() {
                anyhow::bail!("settings must be an object for element {eid}");
            }
            if tree.find(eid).is_none() {
                anyhow::bail!("Element {eid} not found on page {page_id}; no patches were applied");
            }
            ids.push(eid.to_string());
        }

        for (patch, eid) in patches.iter().zip(ids) {
            let new_settings = patch["settings"].clone();
            if !tree.mutate(&eid, |el| merge_settings(&mut el.settings, &new_settings)) {
                anyhow::bail!(
                    "Element {eid} disappeared while preparing page {page_id}; no write was made"
                );
            }
        }

        let jid = wp
            .session
            .as_ref()
            .map(|s| s.record("patch_elements", wp.base_url(), &format!("page:{page_id}")));
        svc.save_tree(page_id, &tree).await?;
        if let (Some(s), Some(id)) = (&wp.session, jid) {
            s.complete(&id);
        }

        Ok(ToolResult::text(format!(
            "Patched {total} elements on page {page_id}"
        )))
    }
}
