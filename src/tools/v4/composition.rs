use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

use super::css_parser::{parse_css_to_variants, variants_to_elementor_json};
use crate::args::{bool_arg, str_arg, u64_arg};
use crate::elementor::generate_id;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct BuildComposition;

#[derive(Debug, Clone)]
pub struct XmlNode {
    pub tag: String,
    pub configuration_id: String,
    pub children: Vec<XmlNode>,
}

#[derive(Debug, Clone)]
pub struct CompiledComposition {
    pub elements: Vec<Value>,
    /// Maps `configuration-id` -> generated Elementor element `id`
    pub id_map: HashMap<String, String>,
    /// Maps `configuration-id` -> global class labels to resolve on the WordPress kit
    pub class_labels: HashMap<String, Vec<String>>,
}

/// Parse `xml_structure` into a tree of `XmlNode`s, enforcing unique `configuration-id`s
/// and rejecting stray text nodes or unsupported attributes.
pub fn parse_xml_structure(xml: &str) -> Result<Vec<XmlNode>> {
    let trimmed = xml.trim();
    if trimmed.is_empty() {
        bail!("xml_structure must not be empty");
    }
    let mut pos = 0usize;
    let mut seen_ids = HashSet::new();
    let mut roots = Vec::new();
    while pos < trimmed.len() {
        skip_ws_and_comments(trimmed, &mut pos)?;
        if pos >= trimmed.len() {
            break;
        }
        roots.push(parse_element(trimmed, &mut pos, &mut seen_ids)?);
    }
    if roots.is_empty() {
        bail!("xml_structure must contain at least one element");
    }
    Ok(roots)
}

fn skip_ws_and_comments(s: &str, pos: &mut usize) -> Result<()> {
    loop {
        while *pos < s.len() {
            let ch = s[*pos..].chars().next().unwrap();
            if ch.is_whitespace() {
                *pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if s[*pos..].starts_with("<!--") {
            let end = s[*pos + 4..]
                .find("-->")
                .ok_or_else(|| anyhow::anyhow!("Unclosed XML comment"))?;
            *pos += 4 + end + 3;
        } else {
            break;
        }
    }
    Ok(())
}

fn parse_element(s: &str, pos: &mut usize, seen_ids: &mut HashSet<String>) -> Result<XmlNode> {
    if !s[*pos..].starts_with('<') {
        bail!("Expected '<' at byte offset {}", *pos);
    }
    *pos += 1;
    if s[*pos..].starts_with('/') {
        bail!("Unexpected closing tag at byte offset {}", *pos - 1);
    }

    let tag_start = *pos;
    while *pos < s.len() {
        let ch = s[*pos..].chars().next().unwrap();
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':' {
            *pos += ch.len_utf8();
        } else {
            break;
        }
    }
    let tag = s[tag_start..*pos].to_string();
    if tag.is_empty() {
        bail!("Empty XML tag name at byte offset {tag_start}");
    }

    let mut configuration_id: Option<String> = None;
    let mut self_closing = false;

    loop {
        while *pos < s.len() {
            let ch = s[*pos..].chars().next().unwrap();
            if ch.is_whitespace() {
                *pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if s[*pos..].starts_with("/>") {
            self_closing = true;
            *pos += 2;
            break;
        }
        if s[*pos..].starts_with('>') {
            *pos += 1;
            break;
        }
        if *pos >= s.len() {
            bail!("Unclosed opening tag <{tag}>");
        }

        // Parse attribute
        let attr_start = *pos;
        while *pos < s.len() {
            let ch = s[*pos..].chars().next().unwrap();
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                *pos += ch.len_utf8();
            } else {
                break;
            }
        }
        let attr_name = &s[attr_start..*pos];
        if attr_name.is_empty() {
            bail!("Invalid attribute in <{tag}> at byte offset {}", *pos);
        }
        while *pos < s.len() && s[*pos..].chars().next().unwrap().is_whitespace() {
            *pos += 1;
        }
        if !s[*pos..].starts_with('=') {
            bail!("Expected '=' after attribute '{attr_name}' in <{tag}>");
        }
        *pos += 1;
        while *pos < s.len() && s[*pos..].chars().next().unwrap().is_whitespace() {
            *pos += 1;
        }
        let quote = s[*pos..]
            .chars()
            .next()
            .filter(|&c| c == '"' || c == '\'')
            .ok_or_else(|| anyhow::anyhow!("Expected quoted attribute value for '{attr_name}'"))?;
        *pos += 1;
        let val_start = *pos;
        let val_end = s[*pos..]
            .find(quote)
            .ok_or_else(|| anyhow::anyhow!("Unclosed attribute value for '{attr_name}'"))?;
        let attr_val = s[val_start..val_start + val_end].to_string();
        *pos += val_end + 1;

        if attr_name != "configuration-id" {
            bail!(
                "Unsupported attribute '{attr_name}' on <{tag}>; only 'configuration-id' is allowed in xml_structure"
            );
        }
        if attr_val.trim().is_empty() {
            bail!("configuration-id on <{tag}> must not be empty");
        }
        if !seen_ids.insert(attr_val.clone()) {
            bail!("Duplicate configuration-id '{attr_val}' in xml_structure");
        }
        configuration_id = Some(attr_val);
    }

    let configuration_id = configuration_id.ok_or_else(|| {
        anyhow::anyhow!("Element <{tag}> is missing required 'configuration-id' attribute")
    })?;

    let mut children = Vec::new();
    if !self_closing {
        loop {
            skip_ws_and_comments(s, pos)?;
            if *pos >= s.len() {
                bail!("Unclosed tag <{tag} configuration-id=\"{configuration_id}\">");
            }
            if s[*pos..].starts_with("</") {
                *pos += 2;
                let close_start = *pos;
                while *pos < s.len() {
                    let ch = s[*pos..].chars().next().unwrap();
                    if ch == '>' || ch.is_whitespace() {
                        break;
                    }
                    *pos += ch.len_utf8();
                }
                let close_tag = &s[close_start..*pos];
                while *pos < s.len() && s[*pos..].chars().next().unwrap().is_whitespace() {
                    *pos += 1;
                }
                if !s[*pos..].starts_with('>') {
                    bail!("Expected '>' in closing tag </{close_tag}>");
                }
                *pos += 1;
                if close_tag != tag {
                    bail!("Mismatched closing tag </{close_tag}> for <{tag}>");
                }
                break;
            }
            if !s[*pos..].starts_with('<') {
                bail!("Text nodes are not allowed inside <{tag}>; pass text via element_config");
            }
            children.push(parse_element(s, pos, seen_ids)?);
        }
    }

    if tag == "e-component" && !children.is_empty() {
        bail!("<e-component> is a leaf tag and must not contain children");
    }

    Ok(XmlNode {
        tag,
        configuration_id,
        children,
    })
}

pub fn compile_composition(
    xml_structure: &str,
    element_config: Option<&Value>,
    classes: Option<&Value>,
    style: Option<&Value>,
    interactions: Option<&Value>,
) -> Result<CompiledComposition> {
    let nodes = parse_xml_structure(xml_structure)?;
    let mut id_map = HashMap::new();
    let mut class_labels = HashMap::new();
    let mut elements = Vec::with_capacity(nodes.len());

    for node in &nodes {
        elements.push(compile_node(
            node,
            false,
            element_config,
            classes,
            style,
            interactions,
            &mut id_map,
            &mut class_labels,
        )?);
    }

    Ok(CompiledComposition {
        elements,
        id_map,
        class_labels,
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_node(
    node: &XmlNode,
    is_inner: bool,
    element_config: Option<&Value>,
    classes: Option<&Value>,
    style: Option<&Value>,
    interactions: Option<&Value>,
    id_map: &mut HashMap<String, String>,
    class_labels: &mut HashMap<String, Vec<String>>,
) -> Result<Value> {
    let el_id = generate_id();
    id_map.insert(node.configuration_id.clone(), el_id.clone());

    let is_container = matches!(
        node.tag.as_str(),
        "e-flexbox" | "e-div-block" | "e-grid" | "container"
    );

    let mut settings = Map::new();
    if let Some(cfg) = element_config.and_then(|v| v.get(&node.configuration_id)) {
        let obj = cfg.as_object().with_context(|| {
            format!(
                "element_config['{}'] must be an object",
                node.configuration_id
            )
        })?;
        for (k, v) in obj {
            settings.insert(k.clone(), v.clone());
        }
    }

    let mut styles = Map::new();
    let mut class_ids: Vec<Value> = Vec::new();

    if let Some(css_val) = style.and_then(|v| v.get(&node.configuration_id)) {
        let css_str = css_val.as_str().with_context(|| {
            format!("style['{}'] must be a CSS string", node.configuration_id)
        })?;
        if !css_str.trim().is_empty() {
            let variants = parse_css_to_variants(css_str)?;
            let style_id = format!("e-{}-{}", el_id, generate_id());
            styles.insert(
                style_id.clone(),
                json!({
                    "id": style_id,
                    "label": "local",
                    "type": "class",
                    "variants": variants_to_elementor_json(&variants),
                }),
            );
            class_ids.push(Value::String(style_id));
        }
    }

    if let Some(cls_val) = classes.and_then(|v| v.get(&node.configuration_id)) {
        let arr = cls_val.as_array().with_context(|| {
            format!("classes['{}'] must be an array of class labels", node.configuration_id)
        })?;
        let mut labels = Vec::with_capacity(arr.len());
        for item in arr {
            let label = item.as_str().with_context(|| {
                format!("classes['{}'] entries must be strings", node.configuration_id)
            })?;
            labels.push(label.to_string());
        }
        if !labels.is_empty() {
            class_labels.insert(node.configuration_id.clone(), labels);
        }
    }

    if !class_ids.is_empty() {
        settings.insert(
            "classes".to_string(),
            json!({
                "$$type": "classes",
                "value": class_ids,
            }),
        );
    }

    let el_interactions = match interactions.and_then(|v| v.get(&node.configuration_id)) {
        Some(Value::Array(arr)) => Value::Array(arr.clone()),
        Some(_) => bail!("interactions['{}'] must be an array", node.configuration_id),
        None => json!([]),
    };

    let mut compiled_children = Vec::with_capacity(node.children.len());
    for child in &node.children {
        compiled_children.push(compile_node(
            child,
            true,
            element_config,
            classes,
            style,
            interactions,
            id_map,
            class_labels,
        )?);
    }

    let mut element = Map::new();
    element.insert("id".to_string(), Value::String(el_id));
    element.insert("configuration_id".to_string(), Value::String(node.configuration_id.clone()));
    if is_container {
        element.insert("elType".to_string(), Value::String(node.tag.clone()));
        element.insert("isInner".to_string(), Value::Bool(is_inner));
    } else {
        element.insert("elType".to_string(), Value::String("widget".to_string()));
        element.insert("widgetType".to_string(), Value::String(node.tag.clone()));
        element.insert("isInner".to_string(), Value::Bool(is_inner));
    }
    element.insert("settings".to_string(), Value::Object(settings));
    element.insert("styles".to_string(), Value::Object(styles));
    element.insert("interactions".to_string(), el_interactions);
    element.insert("elements".to_string(), Value::Array(compiled_children));

    Ok(Value::Object(element))
}

#[async_trait]
impl Tool for BuildComposition {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "build_composition",
            description: "Build a V4 atomic Elementor composition from declarative XML tags (<e-flexbox>, <e-heading>, <e-component>, etc.), element_config, global classes, plain CSS strings (with &:hover, @media(--mobile), var(--label)), and interactions. Supports dry_run validation.",
            input_schema: json!({
                "type": "object",
                "required": ["post_id", "xml_structure"],
                "properties": {
                    "post_id": { "type": "integer", "description": "WordPress post ID of the document to mutate." },
                    "xml_structure": { "type": "string", "description": "Valid XML structure with custom Elementor widget tags. Every element MUST have a unique configuration-id attribute." },
                    "element_config": { "type": "object", "description": "Record mapping configuration-id -> widget settings (or { component_id, overrides } for <e-component>)." },
                    "classes": { "type": "object", "description": "Record mapping configuration-id -> list of global class labels to attach." },
                    "style": { "type": "object", "description": "Record mapping configuration-id -> plain CSS string supporting &:hover/&:focus/&:active and @media(--breakpoint)." },
                    "interactions": { "type": "object", "description": "Record mapping configuration-id -> array of native interaction objects." },
                    "parent_id": { "type": "string", "default": "document", "description": "ID of parent container, or 'document' for root." },
                    "mode": { "type": "string", "enum": ["append", "replace_children"], "default": "append" },
                    "dry_run": { "type": "boolean", "default": false, "description": "If true, validate and return resolved tree without persisting." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let post_id =
            u64_arg(&args, "post_id").ok_or_else(|| anyhow::anyhow!("post_id required"))?;
        let xml_structure = str_arg(&args, "xml_structure")
            .ok_or_else(|| anyhow::anyhow!("xml_structure required"))?;
        let parent_id = str_arg(&args, "parent_id").unwrap_or_else(|| "document".to_string());
        let mode = str_arg(&args, "mode").unwrap_or_else(|| "append".to_string());
        if mode != "append" && mode != "replace_children" {
            bail!("mode must be 'append' or 'replace_children'");
        }
        let dry_run = bool_arg(&args, "dry_run").unwrap_or(false);

        let compiled = compile_composition(
            &xml_structure,
            args.get("element_config"),
            args.get("classes"),
            args.get("style"),
            args.get("interactions"),
        )?;

        if dry_run {
            return Ok(ToolResult::text(serde_json::to_string_pretty(&json!({
                "dry_run": true,
                "post_id": post_id,
                "parent_id": parent_id,
                "mode": mode,
                "id_map": compiled.id_map,
                "class_labels": compiled.class_labels,
                "elements": compiled.elements,
            }))?));
        }

        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/build-composition",
                &json!({
                    "post_id": post_id,
                    "parent_id": parent_id,
                    "mode": mode,
                    "elements": compiled.elements,
                    "id_map": compiled.id_map,
                    "class_labels": compiled.class_labels,
                }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Post(post_id), &mut resp)
            .await;

        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_xml_css_classes_and_interactions() {
        let xml = r#"
            <e-flexbox configuration-id="hero">
                <e-heading configuration-id="title"></e-heading>
                <e-component configuration-id="cta"/>
            </e-flexbox>
        "#;
        let cfg = json!({
            "title": { "title": "Hello" },
            "cta": { "component_id": 42, "overrides": { "label": "Go" } }
        });
        let classes = json!({ "hero": ["section-pad"] });
        let style = json!({
            "hero": "padding: 32px; &:hover { background-color: #fafafa; } @media(--mobile) { padding: 16px; }"
        });
        let interactions = json!({
            "cta": [{ "trigger": "click", "effect": "fade-in" }]
        });

        let compiled = compile_composition(
            xml,
            Some(&cfg),
            Some(&classes),
            Some(&style),
            Some(&interactions),
        )
        .unwrap();

        assert_eq!(compiled.elements.len(), 1);
        let hero = &compiled.elements[0];
        assert_eq!(hero["elType"], "e-flexbox");
        assert_eq!(hero["elements"].as_array().unwrap().len(), 2);
        assert_eq!(hero["elements"][0]["widgetType"], "e-heading");
        assert_eq!(hero["elements"][0]["settings"]["title"], "Hello");
        assert_eq!(hero["elements"][1]["widgetType"], "e-component");
        assert_eq!(hero["elements"][1]["settings"]["component_id"], 42);
        assert_eq!(hero["elements"][1]["interactions"][0]["trigger"], "click");
        assert_eq!(compiled.class_labels["hero"], vec!["section-pad"]);
        assert!(hero["styles"].as_object().unwrap().len() == 1);
    }

    #[test]
    fn rejects_duplicate_configuration_ids_and_text_nodes() {
        assert!(
            parse_xml_structure(r#"<e-flexbox configuration-id="a"><e-heading configuration-id="a"></e-heading></e-flexbox>"#)
                .is_err()
        );
        assert!(
            parse_xml_structure(r#"<e-heading configuration-id="a">No text allowed</e-heading>"#)
                .is_err()
        );
    }
}
