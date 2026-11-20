use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::composition::compile_composition;
use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct ListComponents;
pub struct ManageComponent;

#[async_trait]
impl Tool for ListComponents {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_components",
            description: "List Elementor reusable V4 components on this site. Omit component_ids to list id/name/uid; pass component_ids to fetch overridable_props schemas.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "component_ids": {
                        "type": "array",
                        "description": "Post IDs of the components to inspect for overridable_props.",
                        "items": { "type": "integer" }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let resp = wp
            .post("mcp-for-page-builders/v1/v4/components/list", &args)
            .await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ManageComponent {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "manage_component",
            description: "Create, update, rename, archive, or publish reusable Elementor V4 components (<e-component> compositions) with overridable_props and draft/publish staging.",
            input_schema: json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["create", "update", "rename", "archive", "publish"] },
                    "title": { "type": "string", "minLength": 2, "maxLength": 200 },
                    "component_id": { "type": "integer" },
                    "component_ids": { "type": "array", "items": { "type": "integer" } },
                    "xml_structure": { "type": "string", "description": "XML tree with exactly one root element and unique configuration-ids." },
                    "element_config": { "type": "object" },
                    "classes": { "type": "object" },
                    "style": { "type": "object" },
                    "interactions": { "type": "object" },
                    "source_post_id": { "type": "integer" },
                    "element_id": { "type": "string" },
                    "overridable_props": { "type": "object", "description": "Map override_key -> { target, prop_key, label, group? }." },
                    "publish_status": { "type": "string", "enum": ["publish", "draft"], "default": "publish" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let action = str_arg(&args, "action").ok_or_else(|| anyhow::anyhow!("action required"))?;
        let publish_status = str_arg(&args, "publish_status").unwrap_or_else(|| "publish".into());
        if publish_status != "publish" && publish_status != "draft" {
            bail!("publish_status must be 'publish' or 'draft'");
        }

        let mut payload = json!({
            "action": action,
            "publish_status": publish_status,
        });

        match action.as_str() {
            "create" => {
                let title =
                    str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required for create"))?;
                if !(2..=200).contains(&title.chars().count()) {
                    bail!("title must be 2 to 200 characters");
                }
                payload["title"] = Value::String(title);

                let has_xml = args.get("xml_structure").and_then(Value::as_str).is_some();
                let has_source = args.get("source_post_id").is_some() || args.get("element_id").is_some();
                if has_xml && has_source {
                    bail!("xml_structure and source_post_id/element_id are mutually exclusive");
                }
                if let Some(xml) = str_arg(&args, "xml_structure") {
                    let compiled = compile_composition(
                        &xml,
                        args.get("element_config"),
                        args.get("classes"),
                        args.get("style"),
                        args.get("interactions"),
                    )?;
                    if compiled.elements.len() != 1 {
                        bail!("Component xml_structure must have exactly one root element");
                    }
                    payload["elements"] = Value::Array(compiled.elements);
                    payload["id_map"] = serde_json::to_value(&compiled.id_map)?;
                    payload["class_labels"] = serde_json::to_value(&compiled.class_labels)?;
                } else if has_source {
                    let source_post_id = u64_arg(&args, "source_post_id")
                        .ok_or_else(|| anyhow::anyhow!("source_post_id required with element_id"))?;
                    let element_id = str_arg(&args, "element_id")
                        .ok_or_else(|| anyhow::anyhow!("element_id required with source_post_id"))?;
                    payload["source_post_id"] = json!(source_post_id);
                    payload["element_id"] = Value::String(element_id);
                }
            }
            "update" => {
                let component_id = u64_arg(&args, "component_id")
                    .ok_or_else(|| anyhow::anyhow!("component_id required for update"))?;
                payload["component_id"] = json!(component_id);
                if let Some(xml) = str_arg(&args, "xml_structure") {
                    let compiled = compile_composition(
                        &xml,
                        args.get("element_config"),
                        args.get("classes"),
                        args.get("style"),
                        args.get("interactions"),
                    )?;
                    if compiled.elements.len() != 1 {
                        bail!("Component xml_structure must have exactly one root element");
                    }
                    payload["elements"] = Value::Array(compiled.elements);
                    payload["id_map"] = serde_json::to_value(&compiled.id_map)?;
                    payload["class_labels"] = serde_json::to_value(&compiled.class_labels)?;
                } else if args.get("overridable_props").is_none() {
                    bail!("update without xml_structure requires overridable_props");
                }
            }
            "rename" => {
                let component_id = u64_arg(&args, "component_id")
                    .ok_or_else(|| anyhow::anyhow!("component_id required for rename"))?;
                let title =
                    str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required for rename"))?;
                if !(2..=200).contains(&title.chars().count()) {
                    bail!("title must be 2 to 200 characters");
                }
                payload["component_id"] = json!(component_id);
                payload["title"] = Value::String(title);
            }
            "archive" => {
                let ids = args
                    .get("component_ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("component_ids array required for archive"))?;
                if ids.is_empty() {
                    bail!("component_ids must not be empty");
                }
                payload["component_ids"] = Value::Array(ids.clone());
            }
            "publish" => {
                let component_id = u64_arg(&args, "component_id")
                    .ok_or_else(|| anyhow::anyhow!("component_id required for publish"))?;
                payload["component_id"] = json!(component_id);
            }
            other => bail!("Unknown manage_component action '{other}'"),
        }

        if let Some(props) = args.get("overridable_props") {
            payload["overridable_props"] = props.clone();
        }

        let mut resp = wp
            .post("mcp-for-page-builders/v1/v4/components/manage", &payload)
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}
