use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::wp::WpClient;

/// A single Elementor element (section, container, column, or widget).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: String,
    #[serde(rename = "elType")]
    pub el_type: String,
    #[serde(rename = "widgetType", skip_serializing_if = "Option::is_none")]
    pub widget_type: Option<String>,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

impl Element {
    /// Recursively regenerate IDs for this element and all its children.
    pub fn regenerate_ids(&mut self) {
        self.id = generate_id();
        for child in &mut self.elements {
            child.regenerate_ids();
        }
    }

    /// Merge overlay settings into this element's settings (shallow merge).
    pub fn merge_settings(&mut self, overlay: &Value) {
        if let (Some(base), Some(over)) = (self.settings.as_object_mut(), overlay.as_object()) {
            for (k, v) in over {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

// ── ID generation ─────────────────────────────────────────────────────────────

pub fn generate_id() -> String {
    use rand::Rng;
    format!("{:07x}", rand::rng().random_range(0..0x10000000u32))
}

// ── ElementTree ───────────────────────────────────────────────────────────────

/// Owned tree of Elementor elements with traversal and mutation methods.
pub struct ElementTree(Vec<Element>);

impl ElementTree {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(Self(serde_json::from_str(raw)?))
    }

    pub fn serialize(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(&self.0)?)
    }

    pub fn as_slice(&self) -> &[Element] {
        &self.0
    }

    pub fn find(&self, id: &str) -> Option<Element> {
        find_by_id(&self.0, id)
    }

    pub fn mutate(&mut self, id: &str, f: impl Fn(&mut Element)) -> bool {
        mutate_by_id(&mut self.0, id, &f)
    }

    pub fn remove(&mut self, id: &str) -> Option<Element> {
        remove_by_id(&mut self.0, id)
    }

    /// Insert `el` into `parent_id`'s children at `position` (or root if `parent_id` is None).
    pub fn insert(&mut self, parent_id: Option<&str>, position: usize, el: Element) -> bool {
        match parent_id {
            None => {
                let pos = position.min(self.0.len());
                self.0.insert(pos, el);
                true
            }
            Some(pid) => insert_into_parent(&mut self.0, pid, position, el),
        }
    }

    /// Insert `el` immediately after the element with `after_id`.
    pub fn insert_after(&mut self, after_id: &str, el: Element) -> bool {
        insert_after_id(&mut self.0, after_id, el)
    }

    pub fn search(
        &self,
        widget_type: Option<&str>,
        setting_key: Option<&str>,
        setting_value: Option<&str>,
    ) -> Vec<Element> {
        search_elements(&self.0, widget_type, setting_key, setting_value)
    }

    pub fn flatten(&self) -> Vec<(String, String)> {
        flatten_tree(&self.0, "")
    }
}

// ── ElementorService ──────────────────────────────────────────────────────────

/// Service for Elementor-specific operations that require a WordPress API connection.
pub struct ElementorService<'a> {
    wp: &'a WpClient,
}

impl<'a> ElementorService<'a> {
    pub fn new(wp: &'a WpClient) -> Self {
        Self { wp }
    }

    pub async fn get_tree(&self, page_id: u64) -> anyhow::Result<ElementTree> {
        let page = self.wp.get(&format!("wp/v2/pages/{page_id}?context=edit")).await?;
        let raw = page
            .get("meta")
            .and_then(|m| m.get("_elementor_data"))
            .and_then(|d| d.as_str())
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data on page {page_id}"))?;
        ElementTree::parse(raw)
    }

    pub async fn save_tree(&self, page_id: u64, tree: &ElementTree) -> anyhow::Result<()> {
        let data = tree.serialize()?;
        let body = serde_json::json!({ "meta": { "_elementor_data": data } });
        self.wp.post(&format!("wp/v2/pages/{page_id}"), &body).await?;
        self.clear_cache().await?;
        Ok(())
    }

    /// Clear Elementor CSS cache. Ignores errors — endpoint may not exist on older versions.
    pub async fn clear_cache(&self) -> anyhow::Result<()> {
        let _ = self.wp.delete("elementor/v1/cache").await;
        Ok(())
    }

    /// Set Theme Builder display conditions for a template.
    ///
    /// Elementor Pro requires conditions in TWO places:
    /// 1. Post meta `_elementor_conditions`
    /// 2. WordPress option `elementor_pro_theme_builder_conditions`
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
            .unwrap_or(serde_json::json!({}));

        let mut map = current
            .as_object()
            .cloned()
            .unwrap_or_default();

        let type_entry = map
            .entry(template_type.to_string())
            .or_insert_with(|| serde_json::json!({}));

        if let Some(obj) = type_entry.as_object_mut() {
            obj.insert(template_id.to_string(), Value::Array(cond_values));
        }

        // Best-effort — bridge plugin may not be installed yet
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

// ── Private tree helpers ──────────────────────────────────────────────────────

fn find_by_id(elements: &[Element], id: &str) -> Option<Element> {
    for el in elements {
        if el.id == id {
            return Some(el.clone());
        }
        if let Some(found) = find_by_id(&el.elements, id) {
            return Some(found);
        }
    }
    None
}

fn mutate_by_id(elements: &mut [Element], id: &str, f: &dyn Fn(&mut Element)) -> bool {
    for el in elements.iter_mut() {
        if el.id == id {
            f(el);
            return true;
        }
        if mutate_by_id(&mut el.elements, id, f) {
            return true;
        }
    }
    false
}

fn remove_by_id(elements: &mut Vec<Element>, id: &str) -> Option<Element> {
    if let Some(pos) = elements.iter().position(|e| e.id == id) {
        return Some(elements.remove(pos));
    }
    for el in elements.iter_mut() {
        if let Some(removed) = remove_by_id(&mut el.elements, id) {
            return Some(removed);
        }
    }
    None
}

fn insert_into_parent(elements: &mut [Element], parent_id: &str, position: usize, new_el: Element) -> bool {
    for el in elements.iter_mut() {
        if el.id == parent_id {
            let pos = position.min(el.elements.len());
            el.elements.insert(pos, new_el);
            return true;
        }
        if insert_into_parent(&mut el.elements, parent_id, position, new_el.clone()) {
            return true;
        }
    }
    false
}

fn insert_after_id(elements: &mut Vec<Element>, after_id: &str, new_el: Element) -> bool {
    if let Some(pos) = elements.iter().position(|e| e.id == after_id) {
        elements.insert(pos + 1, new_el);
        return true;
    }
    for el in elements.iter_mut() {
        if insert_after_id(&mut el.elements, after_id, new_el.clone()) {
            return true;
        }
    }
    false
}

fn search_elements(
    elements: &[Element],
    widget_type: Option<&str>,
    setting_key: Option<&str>,
    setting_value: Option<&str>,
) -> Vec<Element> {
    let mut results = Vec::new();
    for el in elements {
        let type_match = widget_type.is_none_or(|wt| el.widget_type.as_deref() == Some(wt));
        let setting_match = match (setting_key, setting_value) {
            (Some(k), Some(v)) => el.settings.get(k).and_then(|sv| sv.as_str()) == Some(v),
            (Some(k), None) => el.settings.get(k).is_some(),
            _ => true,
        };
        if type_match && setting_match {
            results.push(el.clone());
        }
        results.extend(search_elements(&el.elements, widget_type, setting_key, setting_value));
    }
    results
}

fn flatten_tree(elements: &[Element], prefix: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, el) in elements.iter().enumerate() {
        let path = if prefix.is_empty() {
            format!("[{i}]")
        } else {
            format!("{prefix}[{i}]")
        };
        let label = match &el.widget_type {
            Some(wt) => format!("{} ({})", el.el_type, wt),
            None => el.el_type.clone(),
        };
        out.push((path.clone(), format!("{label} id={}", el.id)));
        out.extend(flatten_tree(&el.elements, &path));
    }
    out
}
