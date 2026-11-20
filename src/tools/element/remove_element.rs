use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct RemoveElement;

#[async_trait]
impl Tool for RemoveElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "remove_element",
            description: "Remove an element by ID from a page's element tree.",
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
        match tree.remove(&eid) {
            Some(_) => {
                let cache = svc.save_tree(page_id, &tree).await?;
                Ok(ToolResult::text(format!(
                    "Removed element {eid} from page {page_id}{}",
                    cache.summary_line()
                )))
            }
            None => Ok(ToolResult::error(format!(
                "Element {eid} not found on page {page_id}"
            ))),
        }
    }
}
