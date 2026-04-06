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

// ── ID generation ─────────────────────────────────────────────────────────────

pub fn generate_id() -> String {
    use rand::Rng;
    format!("{:07x}", rand::rng().random_range(0..0x10000000u32))
}

// ── Parse / serialize ─────────────────────────────────────────────────────────

pub fn parse_data(raw: &str) -> anyhow::Result<Vec<Element>> {
    Ok(serde_json::from_str(raw)?)
}

pub fn serialize_data(elements: &[Element]) -> anyhow::Result<String> {
    Ok(serde_json::to_string(elements)?)
}

// ── Page-level helpers (read/write elementor data via WP API) ─────────────────

pub async fn get_page_elements(wp: &WpClient, page_id: u64) -> anyhow::Result<Vec<Element>> {
    let page = wp.get(&format!("wp/v2/pages/{page_id}?context=edit")).await?;
    let raw = page.get("meta")
        .and_then(|m| m.get("_elementor_data"))
        .and_then(|d| d.as_str())
        .ok_or_else(|| anyhow::anyhow!("No _elementor_data on page {page_id}"))?;
    parse_data(raw)
}

pub async fn set_page_elements(wp: &WpClient, page_id: u64, elements: &[Element]) -> anyhow::Result<()> {
    let data = serialize_data(elements)?;
    let body = serde_json::json!({ "meta": { "_elementor_data": data } });
    wp.post(&format!("wp/v2/pages/{page_id}"), &body).await?;
    wp.clear_elementor_cache().await?;
    Ok(())
}

// ── Tree traversal ────────────────────────────────────────────────────────────

pub fn find_by_id(elements: &[Element], id: &str) -> Option<Element> {
    for el in elements {
        if el.id == id { return Some(el.clone()); }
        if let Some(found) = find_by_id(&el.elements, id) { return Some(found); }
    }
    None
}

pub fn mutate_by_id(elements: &mut [Element], id: &str, f: &dyn Fn(&mut Element)) -> bool {
    for el in elements.iter_mut() {
        if el.id == id { f(el); return true; }
        if mutate_by_id(&mut el.elements, id, f) { return true; }
    }
    false
}

pub fn remove_by_id(elements: &mut Vec<Element>, id: &str) -> Option<Element> {
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

/// Insert element into a parent's children (or root if parent_id is None).
pub fn insert_at(elements: &mut Vec<Element>, parent_id: Option<&str>, position: usize, new_el: Element) -> bool {
    match parent_id {
        None => {
            let pos = position.min(elements.len());
            elements.insert(pos, new_el);
            true
        }
        Some(pid) => insert_into_parent(elements, pid, position, new_el),
    }
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

pub fn regenerate_ids(el: &mut Element) {
    el.id = generate_id();
    for child in &mut el.elements {
        regenerate_ids(child);
    }
}

/// Search elements by widget type and/or setting value.
pub fn search(elements: &[Element], widget_type: Option<&str>, setting_key: Option<&str>, setting_value: Option<&str>) -> Vec<Element> {
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
        results.extend(search(&el.elements, widget_type, setting_key, setting_value));
    }
    results
}

/// Flatten tree into (path, summary) pairs.
pub fn flatten_tree(elements: &[Element], prefix: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, el) in elements.iter().enumerate() {
        let path = if prefix.is_empty() { format!("[{i}]") } else { format!("{prefix}[{i}]") };
        let label = match &el.widget_type {
            Some(wt) => format!("{} ({})", el.el_type, wt),
            None => el.el_type.clone(),
        };
        out.push((path.clone(), format!("{label} id={}", el.id)));
        out.extend(flatten_tree(&el.elements, &path));
    }
    out
}

/// Merge settings: overlay new values onto existing settings object.
pub fn merge_settings(base: &mut Value, overlay: &Value) {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in overlay_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }
}
