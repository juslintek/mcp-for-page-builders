use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct UpdateElement;

#[async_trait]
impl Tool for UpdateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_element",
            description: "Update an element's settings by ID. Merges provided settings with existing ones (partial update). Only the keys you provide are changed.\n\nWorkflow: Use css_to_elementor output as the settings input. Verify changes with screenshot_page or visual_diff afterward.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "expected_revision", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "expected_revision": { "type": "string", "description": "Revision returned by get_element_tree" },
                    "element_id": { "type": "string" },
                    "settings": { "type": "object", "description": "Settings to merge into the element" },
                    "interactions": { "type": "array", "description": "Optional native Elementor V4 interactions array to set on the element (pass [] to clear).", "items": { "type": "object" } }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id =
            u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid =
            str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let new_settings = args.get("settings").cloned();
        let new_interactions = args.get("interactions").cloned();
        if new_settings.is_none() && new_interactions.is_none() {
            anyhow::bail!("settings or interactions required");
        }
        if let Some(ref inter) = new_interactions
            && !inter.is_array()
        {
            anyhow::bail!("interactions must be an array");
        }

        let expected_revision =
            crate::args::str_arg(&args, "expected_revision").ok_or_else(|| {
                anyhow::anyhow!("expected_revision required; read the page tree first")
            })?;
        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        ElementorService::require_revision(&tree, &expected_revision)?;
        let found = tree.mutate(&eid, |el| {
            if let Some(ref s) = new_settings {
                crate::elementor::merge_settings(&mut el.settings, s);
            }
            if let Some(ref inter) = new_interactions {
                el.extra.insert("interactions".to_string(), inter.clone());
            }
        });

        if !found {
            return Ok(ToolResult::error(format!(
                "Element {eid} not found on page {page_id}"
            )));
        }

        let jid = wp.session.as_ref().map(|s| {
            s.record(
                "update_element",
                wp.base_url(),
                &format!("page:{page_id}/el:{eid}"),
            )
        });
        svc.save_tree(page_id, &tree).await?;
        if let (Some(s), Some(id)) = (&wp.session, jid) {
            s.complete(&id);
        }
        Ok(ToolResult::text(format!(
            "Updated settings for element {eid} on page {page_id}"
        )))
    }
}
