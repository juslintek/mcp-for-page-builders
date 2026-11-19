use serde_json::Value;
use std::fmt;

use crate::types::Element;

pub fn generate_id() -> String {
    use rand::Rng;
    format!("{:07x}", rand::rng().random_range(0..0x1000_0000_u32))
}

pub fn find_by_id(elements: &[Element], id: &str) -> Option<Element> {
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

pub fn mutate_by_id(elements: &mut [Element], id: &str, f: &dyn Fn(&mut Element)) -> bool {
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

pub fn insert_at(
    elements: &mut Vec<Element>,
    parent_id: Option<&str>,
    position: usize,
    el: Element,
) -> bool {
    match parent_id {
        None => {
            let pos = position.min(elements.len());
            elements.insert(pos, el);
            true
        }
        Some(pid) => insert_into_parent(elements, pid, position, el),
    }
}

pub fn merge_settings(base: &mut Value, overlay: &Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

pub fn regenerate_ids(el: &mut Element) {
    el.id = generate_id();
    for child in &mut el.elements {
        regenerate_ids(child);
    }
}

pub fn search(
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
        results.extend(search(
            &el.elements,
            widget_type,
            setting_key,
            setting_value,
        ));
    }
    results
}

pub fn flatten_tree(elements: &[Element], prefix: &str) -> Vec<(String, String)> {
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

pub fn parse_data(raw: &str) -> anyhow::Result<Vec<Element>> {
    Ok(serde_json::from_str(raw)?)
}

pub fn serialize_data(elements: &[Element]) -> anyhow::Result<String> {
    Ok(serde_json::to_string(elements)?)
}

#[derive(Debug, Clone)]
pub struct ElementorDocument {
    pub raw: String,
    pub revision: String,
}

#[derive(Debug)]
pub struct DocumentPersistedError(String);

impl fmt::Display for DocumentPersistedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DocumentPersistedError {}

pub async fn get_page_document(
    wp: &crate::wp::WpClient,
    page_id: u64,
) -> anyhow::Result<ElementorDocument> {
    let bridge_ep = format!("mcp-for-page-builders/v1/document/{page_id}");
    if let Ok(value) = wp.get(&bridge_ep).await
        && let (Some(raw), Some(revision)) =
            (value["document"].as_str(), value["revision"].as_str())
    {
        return Ok(ElementorDocument {
            raw: raw.to_string(),
            revision: revision.to_string(),
        });
    }
    for ep in [
        format!("wp/v2/pages/{page_id}?context=edit"),
        format!("wp/v2/posts/{page_id}?context=edit"),
        format!("wp/v2/elementor_library/{page_id}?context=edit"),
        format!("wp/v2/udesign_template/{page_id}?context=edit"),
    ] {
        if let Ok(page) = wp.get(&ep).await
            && let Some(raw) = page
                .get("meta")
                .and_then(|m| m.get("_elementor_data"))
                .and_then(Value::as_str)
        {
            return Ok(ElementorDocument {
                raw: raw.to_string(),
                revision: format!("legacy:{raw}"),
            });
        }
    }
    let bridge_meta = format!("mcp-for-page-builders/v1/postmeta/{page_id}/_elementor_data");
    if let Ok(value) = wp.get(&bridge_meta).await
        && let Some(raw) = value.get("value").and_then(Value::as_str)
    {
        return Ok(ElementorDocument {
            raw: raw.to_string(),
            revision: format!("legacy:{raw}"),
        });
    }
    anyhow::bail!(
        "No editable _elementor_data found for post {page_id}; verify edit_post permission and install the MCP bridge for checked compare-and-set writes"
    )
}

pub async fn get_page_elements(
    wp: &crate::wp::WpClient,
    page_id: u64,
) -> anyhow::Result<Vec<Element>> {
    let document = get_page_document(wp, page_id).await?;
    parse_data(&document.raw)
}

pub async fn set_page_elements_checked(
    wp: &crate::wp::WpClient,
    page_id: u64,
    elements: &[Element],
    expected_revision: &str,
    operation_id: &str,
) -> anyhow::Result<()> {
    validate_document(elements)?;
    let data = serialize_data(elements)?;
    let typed: Value = serde_json::from_str(&data)?;
    let bridge_ep = format!("mcp-for-page-builders/v1/document/{page_id}/apply");
    if expected_revision.starts_with("legacy:") {
        anyhow::bail!(
            "Checked Elementor writes require the MCP bridge compare-and-set endpoint; install the bridge before editing page {page_id}"
        );
    }
    let response = wp
        .post(
            &bridge_ep,
            &serde_json::json!({
                "expected_revision": expected_revision,
                "operation_id": operation_id,
                "document": typed,
            }),
        )
        .await?;
    if response["saved"] != true || response["readback_verified"] != true {
        anyhow::bail!(
            "Elementor save returned an incomplete result: {}",
            serde_json::to_string(&response)?
        );
    }
    if response["cache_regenerated"] != true {
        return Err(DocumentPersistedError(format!(
            "Elementor document was saved and verified, but CSS regeneration failed: {}",
            response["error"].as_str().unwrap_or("no error detail")
        ))
        .into());
    }
    Ok(())
}

fn validate_document(elements: &[Element]) -> anyhow::Result<()> {
    fn visit(
        elements: &[Element],
        ids: &mut std::collections::HashSet<String>,
    ) -> anyhow::Result<()> {
        for element in elements {
            if element.id.trim().is_empty() {
                anyhow::bail!("Every Elementor node needs a non-empty id; no write was made");
            }
            if !ids.insert(element.id.clone()) {
                anyhow::bail!(
                    "Duplicate Elementor node id '{}'; no write was made",
                    element.id
                );
            }
            if !matches!(
                element.el_type.as_str(),
                "container" | "column" | "section" | "widget"
            ) && !element.el_type.starts_with("e-")
            {
                anyhow::bail!(
                    "Unsupported Elementor elType '{}' for node {}; no write was made",
                    element.el_type,
                    element.id
                );
            }
            if element.el_type == "widget"
                && element.widget_type.as_deref().is_none_or(str::is_empty)
            {
                anyhow::bail!(
                    "Widget node {} has no widgetType; no write was made",
                    element.id
                );
            }
            if !element.settings.is_object() {
                anyhow::bail!(
                    "Element settings must be an object on node {}; no write was made",
                    element.id
                );
            }
            visit(&element.elements, ids)?;
        }
        Ok(())
    }
    visit(elements, &mut std::collections::HashSet::new())
}

#[cfg(test)]
mod tests {
    use super::validate_document;
    use crate::types::Element;
    use serde_json::json;
    use std::collections::HashMap;

    fn element(id: &str, el_type: &str, children: Vec<Element>) -> Element {
        Element {
            id: id.into(),
            el_type: el_type.into(),
            widget_type: (el_type == "widget").then(|| "heading".into()),
            settings: json!({}),
            elements: children,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn document_validation_checks_nested_duplicate_ids() {
        let tree = vec![element(
            "same",
            "container",
            vec![element("same", "widget", vec![])],
        )];
        assert!(validate_document(&tree).is_err());
    }

    #[test]
    fn document_validation_rejects_nodes_without_ids() {
        let tree = vec![element(" ", "container", vec![])];
        assert!(validate_document(&tree).is_err());
    }
}

pub(crate) fn insert_into_parent(
    elements: &mut [Element],
    parent_id: &str,
    position: usize,
    new_el: Element,
) -> bool {
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

pub(crate) fn insert_after_id(
    elements: &mut Vec<Element>,
    after_id: &str,
    new_el: Element,
) -> bool {
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
