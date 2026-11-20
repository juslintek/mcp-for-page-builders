use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::css_parser::{parse_css_to_variants, variants_to_elementor_json};
use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct ManageGlobalVariable;
pub struct ManageClasses;
pub struct ReorderClasses;
pub struct GetDefaultStyles;
pub struct ManageDefaultStyles;

#[async_trait]
impl Tool for ManageGlobalVariable {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "manage_global_variable",
            description: "Manage V4 global variables (color, font, size, custom-size). Bulk create, update, or delete on the active kit (up to 50 operations).",
            input_schema: json!({
                "type": "object",
                "required": ["operations"],
                "properties": {
                    "operations": {
                        "type": "array",
                        "description": "Bulk operations (1-50). Each item requires action; create needs type/label/value, update needs id/label/value, delete needs id.",
                        "items": {
                            "type": "object",
                            "required": ["action"],
                            "properties": {
                                "action": { "type": "string", "enum": ["create", "update", "delete"] },
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "type": { "type": "string", "enum": ["global-color-variable", "global-font-variable", "global-size-variable", "global-custom-size-variable"] },
                                "value": { "type": "string" }
                            }
                        }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let ops = args
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("operations array required"))?;
        if ops.is_empty() || ops.len() > 50 {
            bail!("operations must contain between 1 and 50 items");
        }
        for (idx, op) in ops.iter().enumerate() {
            let action = op
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("operations[{idx}].action required"))?;
            match action {
                "create" => {
                    if op.get("type").and_then(Value::as_str).is_none()
                        || op.get("label").and_then(Value::as_str).is_none()
                        || op.get("value").and_then(Value::as_str).is_none()
                    {
                        bail!("operations[{idx}] create requires type, label, and value");
                    }
                }
                "update" => {
                    if op.get("id").and_then(Value::as_str).is_none() {
                        bail!("operations[{idx}] update requires id");
                    }
                }
                "delete" => {
                    if op.get("id").and_then(Value::as_str).is_none() {
                        bail!("operations[{idx}] delete requires id");
                    }
                }
                other => bail!("operations[{idx}] unknown action '{other}'"),
            }
        }

        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/variables",
                &json!({ "operations": ops }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ManageClasses {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "manage_classes",
            description: "Manage V4 global CSS classes on the active kit. Bulk create, update, or delete using raw CSS declarations (up to 50 operations). Supports &:hover/&:focus/&:active and @media(--breakpoint).",
            input_schema: json!({
                "type": "object",
                "required": ["operations"],
                "properties": {
                    "operations": {
                        "type": "array",
                        "description": "Bulk operations (1-50). create needs label and css. update/delete need id or label.",
                        "items": {
                            "type": "object",
                            "required": ["action"],
                            "properties": {
                                "action": { "type": "string", "enum": ["create", "update", "delete"] },
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "css": { "type": "string", "description": "Plain CSS string. Supports &:hover/&:focus/&:active and @media(--breakpoint). In patch mode: prop: null removes prop; all: null wipes variant." },
                                "mode": { "type": "string", "enum": ["patch", "replace"], "default": "patch" }
                            }
                        }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let ops = args
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("operations array required"))?;
        let prepared = prepare_class_operations(ops)?;
        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/classes",
                &json!({ "operations": prepared }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

pub fn prepare_class_operations(ops: &[Value]) -> Result<Vec<Value>> {
    if ops.is_empty() || ops.len() > 50 {
        bail!("operations must contain between 1 and 50 items");
    }
    let mut prepared = Vec::with_capacity(ops.len());
    for (idx, op) in ops.iter().enumerate() {
        let action = op
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("operations[{idx}].action required"))?;
        let mut out = op.clone();
        match action {
            "create" => {
                if op.get("label").and_then(Value::as_str).is_none() {
                    bail!("operations[{idx}] create requires label");
                }
                let css = op
                    .get("css")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("operations[{idx}] create requires css"))?;
                let variants = parse_css_to_variants(css)?;
                out["variants"] = Value::Array(variants_to_elementor_json(&variants));
            }
            "update" => {
                if op.get("id").and_then(Value::as_str).is_none()
                    && op.get("label").and_then(Value::as_str).is_none()
                {
                    bail!("operations[{idx}] update requires id or label");
                }
                if let Some(css) = op.get("css").and_then(Value::as_str) {
                    let variants = parse_css_to_variants(css)?;
                    out["variants"] = Value::Array(variants_to_elementor_json(&variants));
                    let clear_breakpoints: Vec<Value> = variants
                        .iter()
                        .filter(|v| v.clear_all)
                        .map(|v| json!({ "breakpoint": v.meta.breakpoint, "state": v.meta.state }))
                        .collect();
                    if !clear_breakpoints.is_empty() {
                        out["clear_variants"] = Value::Array(clear_breakpoints);
                    }
                }
            }
            "delete" => {
                if op.get("id").and_then(Value::as_str).is_none()
                    && op.get("label").and_then(Value::as_str).is_none()
                {
                    bail!("operations[{idx}] delete requires id or label");
                }
            }
            other => bail!("operations[{idx}] unknown action '{other}'"),
        }
        prepared.push(out);
    }
    Ok(prepared)
}

#[async_trait]
impl Tool for ReorderClasses {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "reorder_classes",
            description: "Change the priority order of V4 global CSS classes on the active kit. Classes earlier in the order override conflicting CSS declarations from classes later in the order.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "moves": {
                        "type": "array",
                        "description": "One to fifty sequential moves.",
                        "items": {
                            "type": "object",
                            "required": ["id", "position"],
                            "properties": {
                                "id": { "type": "string" },
                                "position": { "type": "string", "enum": ["before", "after", "start", "end"] },
                                "ref": { "type": "string" }
                            }
                        }
                    },
                    "order": {
                        "type": "array",
                        "description": "Optional complete or partial priority order of class IDs.",
                        "items": { "type": "string" }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let has_moves = args.get("moves").and_then(Value::as_array).is_some();
        let has_order = args.get("order").and_then(Value::as_array).is_some();
        if !has_moves && !has_order {
            bail!("Either moves or order must be provided");
        }
        if let Some(moves) = args.get("moves").and_then(Value::as_array) {
            if moves.is_empty() || moves.len() > 50 {
                bail!("moves must contain between 1 and 50 items");
            }
            for (idx, m) in moves.iter().enumerate() {
                let pos = m
                    .get("position")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("moves[{idx}].position required"))?;
                if matches!(pos, "before" | "after")
                    && m.get("ref").and_then(Value::as_str).is_none()
                {
                    bail!("moves[{idx}] with position '{pos}' requires ref");
                }
            }
        }
        let mut resp = wp
            .post("mcp-for-page-builders/v1/v4/classes/reorder", &args)
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for GetDefaultStyles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_default_styles",
            description: "Returns the active kit's site-wide default styles for a single HTML wrapper tag (h1..h6, p, a, section, div, button, ...) as a raw CSS string.",
            input_schema: json!({
                "type": "object",
                "required": ["tag"],
                "properties": {
                    "tag": { "type": "string", "description": "HTML wrapper tag to look up (e.g. h2, p, a, button, section)." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let tag = str_arg(&args, "tag").ok_or_else(|| anyhow::anyhow!("tag required"))?;
        if !is_valid_wrapper_tag(&tag) {
            bail!("Invalid HTML wrapper tag '{tag}'");
        }
        let resp = wp
            .get(&format!(
                "mcp-for-page-builders/v1/v4/default-styles?tag={tag}"
            ))
            .await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ManageDefaultStyles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "manage_default_styles",
            description: "Bulk manage the active kit's site-wide default styles keyed by HTML wrapper tag (h1..h6, p, a, section, div, ...).",
            input_schema: json!({
                "type": "object",
                "required": ["operations"],
                "properties": {
                    "operations": {
                        "type": "array",
                        "description": "Bulk operations (1-20). Each item requires action and tag. update needs css.",
                        "items": {
                            "type": "object",
                            "required": ["action", "tag"],
                            "properties": {
                                "action": { "type": "string", "enum": ["update", "delete"] },
                                "tag": { "type": "string" },
                                "css": { "type": "string" },
                                "mode": { "type": "string", "enum": ["patch", "replace"], "default": "patch" }
                            }
                        }
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let ops = args
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("operations array required"))?;
        if ops.is_empty() || ops.len() > 20 {
            bail!("operations must contain between 1 and 20 items");
        }
        let mut prepared = Vec::with_capacity(ops.len());
        for (idx, op) in ops.iter().enumerate() {
            let action = op
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("operations[{idx}].action required"))?;
            let tag = op
                .get("tag")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("operations[{idx}].tag required"))?;
            if !is_valid_wrapper_tag(tag) {
                bail!("operations[{idx}] invalid HTML wrapper tag '{tag}'");
            }
            let mut out = op.clone();
            match action {
                "update" => {
                    let css = op
                        .get("css")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("operations[{idx}] update requires css"))?;
                    let variants = parse_css_to_variants(css)?;
                    out["variants"] = Value::Array(variants_to_elementor_json(&variants));
                }
                "delete" => {}
                other => bail!("operations[{idx}] unknown action '{other}'"),
            }
            prepared.push(out);
        }
        let mut resp = wp
            .post(
                "mcp-for-page-builders/v1/v4/default-styles",
                &json!({ "operations": prepared }),
            )
            .await?;
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Global, &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

fn is_valid_wrapper_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 16
        && tag
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
