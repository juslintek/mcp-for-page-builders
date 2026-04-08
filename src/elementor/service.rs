use serde_json::Value;

use crate::wp::WpClient;
use super::ops::{get_page_elements, set_page_elements};
use super::tree::ElementTree;

/// Service for Elementor-specific operations that require a `WordPress` API connection.
pub struct ElementorService<'a> {
    wp: &'a WpClient,
}

impl<'a> ElementorService<'a> {
    pub const fn new(wp: &'a WpClient) -> Self {
        Self { wp }
    }

    pub async fn get_tree(&self, page_id: u64) -> anyhow::Result<ElementTree> {
        let elements = get_page_elements(self.wp, page_id).await?;
        Ok(ElementTree(elements))
    }

    pub async fn save_tree(&self, page_id: u64, tree: &ElementTree) -> anyhow::Result<()> {
        set_page_elements(self.wp, page_id, &tree.0).await
    }

    pub async fn clear_cache(&self) -> anyhow::Result<()> {
        self.wp.clear_elementor_cache().await
    }

    pub async fn set_template_conditions(
        &self,
        template_id: u64,
        template_type: &str,
        conditions: &[String],
    ) -> anyhow::Result<()> {
        let cond_values: Vec<Value> = conditions.iter().map(|c| Value::String(c.clone())).collect();

        self.wp
            .post(
                &format!("wp/v2/elementor_library/{template_id}"),
                &serde_json::json!({ "meta": { "_elementor_conditions": cond_values } }),
            )
            .await?;

        let current = self
            .wp
            .get("elementor-mcp/v1/option/elementor_pro_theme_builder_conditions")
            .await
            .unwrap_or_else(|_| serde_json::json!({}));

        let mut map = current.as_object().cloned().unwrap_or_default();

        let type_entry = map
            .entry(template_type.to_string())
            .or_insert_with(|| serde_json::json!({}));

        if let Some(obj) = type_entry.as_object_mut() {
            obj.insert(template_id.to_string(), Value::Array(cond_values));
        }

        self.wp
            .post(
                "elementor-mcp/v1/option/elementor_pro_theme_builder_conditions",
                &Value::Object(map),
            )
            .await
            .ok();

        self.clear_cache().await?;
        Ok(())
    }
}
