use serde_json::Value;

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

pub fn insert_at(elements: &mut Vec<Element>, parent_id: Option<&str>, position: usize, el: Element) -> bool {
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
        results.extend(search(&el.elements, widget_type, setting_key, setting_value));
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

pub async fn get_page_elements(wp: &crate::wp::WpClient, page_id: u64) -> anyhow::Result<Vec<Element>> {
    // Try REST API endpoints first
    let endpoints = [
        format!("wp/v2/pages/{page_id}?context=edit"),
        format!("wp/v2/posts/{page_id}?context=edit"),
        format!("wp/v2/elementor_library/{page_id}?context=edit"),
        format!("wp/v2/udesign_template/{page_id}?context=edit"),
    ];
    for ep in &endpoints {
        if let Ok(page) = wp.get(ep).await {
            if let Some(raw) = page.get("meta").and_then(|m| m.get("_elementor_data")).and_then(|d| d.as_str()) {
                if !raw.is_empty() { return parse_data(raw); }
            }
        }
    }
    // Fall back to bridge postmeta endpoint
    let bridge_ep = format!("mcp-for-page-builders/v1/postmeta/{page_id}/_elementor_data");
    if let Ok(val) = wp.get(&bridge_ep).await {
        let raw = val.get("value").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data for post {page_id}"))?;
        return parse_data(raw);
    }
    anyhow::bail!("No _elementor_data found for post {page_id}")
}

pub async fn set_page_elements(wp: &crate::wp::WpClient, page_id: u64, elements: &[Element]) -> anyhow::Result<()> {
    let data = serialize_data(elements)?;
    let body = serde_json::json!({ "meta": { "_elementor_data": data } });
    let endpoints = [
        format!("wp/v2/pages/{page_id}"),
        format!("wp/v2/posts/{page_id}"),
        format!("wp/v2/elementor_library/{page_id}"),
        format!("wp/v2/udesign_template/{page_id}"),
    ];
    for ep in &endpoints {
        if wp.post(ep, &body).await.is_ok() {
            let _ = wp.delete("elementor/v1/cache").await;
            return Ok(());
        }
    }
    // Fall back to bridge postmeta endpoint
    let bridge_ep = format!("mcp-for-page-builders/v1/postmeta/{page_id}/_elementor_data");
    wp.post(&bridge_ep, &serde_json::json!({"value": data})).await?;
    let _ = wp.delete("elementor/v1/cache").await;
    Ok(())
}

pub(crate) fn insert_into_parent(elements: &mut [Element], parent_id: &str, position: usize, new_el: Element) -> bool {
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

pub(crate) fn insert_after_id(elements: &mut Vec<Element>, after_id: &str, new_el: Element) -> bool {
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
