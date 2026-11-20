use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct DuplicateElement;

#[async_trait]
impl Tool for DuplicateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "duplicate_element",
            description: "Clone an element (and all children) with new IDs, inserted right after the original.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "expected_revision", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "expected_revision": { "type": "string", "description": "Revision returned by get_element_tree" },
                    "element_id": { "type": "string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id =
            u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid =
            str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let expected_revision =
            crate::args::str_arg(&args, "expected_revision").ok_or_else(|| {
                anyhow::anyhow!("expected_revision required; read the page tree first")
            })?;
        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        ElementorService::require_revision(&tree, &expected_revision)?;

        let mut clone = tree
            .find(&eid)
            .ok_or_else(|| anyhow::anyhow!("Element {eid} not found"))?;
        crate::elementor::regenerate_ids(&mut clone);
        let clone_id = clone.id.clone();

        if !tree.insert_after(&eid, clone) {
            return Ok(ToolResult::error(format!(
                "Could not insert clone after {eid}"
            )));
        }

        let cache = svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!(
            "Duplicated {eid} → {clone_id} on page {page_id}{}",
            cache.summary_line()
        )))
    }
}
