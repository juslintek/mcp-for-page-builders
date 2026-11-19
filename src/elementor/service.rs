use serde_json::Value;

use super::ops::{get_page_document, set_page_elements_checked};
use super::tree::ElementTree;
use crate::wp::WpClient;

/// Service for Elementor-specific operations that require a `WordPress` API connection.
pub struct ElementorService<'a> {
    wp: &'a WpClient,
}

impl<'a> ElementorService<'a> {
    pub const fn new(wp: &'a WpClient) -> Self {
        Self { wp }
    }

    pub async fn get_tree(&self, page_id: u64) -> anyhow::Result<ElementTree> {
        let document = get_page_document(self.wp, page_id).await?;
        Ok(ElementTree {
            elements: super::ops::parse_data(&document.raw)?,
            revision: Some(document.revision),
        })
    }

    pub async fn save_tree(&self, page_id: u64, tree: &ElementTree) -> anyhow::Result<()> {
        let revision = tree.revision.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Page revision is unavailable; read the page tree again")
        })?;
        set_page_elements_checked(
            self.wp,
            page_id,
            &tree.elements,
            revision,
            &format!("mcp-{}", super::ops::generate_id()),
        )
        .await
    }

    pub fn require_revision(tree: &ElementTree, expected: &str) -> anyhow::Result<()> {
        let actual = tree.revision.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Page revision is unavailable; read the page tree again")
        })?;
        if actual != expected {
            anyhow::bail!(
                "Stale page revision: expected {expected}, current revision is {actual}; read the page tree again"
            );
        }
        Ok(())
    }

    pub async fn save_elements_checked(
        &self,
        page_id: u64,
        elements: &[crate::types::Element],
        revision: &str,
        operation_id: &str,
    ) -> anyhow::Result<()> {
        set_page_elements_checked(self.wp, page_id, elements, revision, operation_id).await
    }

    pub async fn restore_tree(
        &self,
        page_id: u64,
        operation_id: &str,
        expected_revision: &str,
    ) -> anyhow::Result<Value> {
        let result = self
            .wp
            .post(
                &format!("mcp-for-page-builders/v1/document/{page_id}/restore"),
                &serde_json::json!({
                    "operation_id": operation_id,
                    "expected_revision": expected_revision,
                }),
            )
            .await?;
        if result["restored"] != true || result["readback_verified"] != true {
            anyhow::bail!(
                "Restore returned an incomplete result: {}",
                serde_json::to_string(&result)?
            );
        }
        Ok(result)
    }

    pub async fn capabilities(&self) -> anyhow::Result<Value> {
        self.wp.get("mcp-for-page-builders/v1/capabilities").await
    }

    pub async fn runtime(&self) -> anyhow::Result<Value> {
        self.wp.get("mcp-for-page-builders/v1/runtime").await
    }

    pub async fn element_schema(&self, widget_type: &str) -> anyhow::Result<Value> {
        self.wp
            .get(&format!(
                "mcp-for-page-builders/v1/element-schema/{widget_type}"
            ))
            .await
    }

    pub async fn style_schema(&self) -> anyhow::Result<Value> {
        self.wp.get("mcp-for-page-builders/v1/style-schema").await
    }

    pub async fn clear_cache(&self) -> anyhow::Result<()> {
        self.wp.clear_elementor_cache().await
    }
}
