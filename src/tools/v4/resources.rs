use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct ListResources;
pub struct ReadResource;

pub fn static_resource_catalog() -> Vec<Value> {
    vec![
        json!({
            "uri": "elementor://global-classes",
            "name": "Global Classes",
            "description": "Active kit V4 global CSS classes and their priority order.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "elementor://global-variables",
            "name": "Global Variables",
            "description": "Active kit V4 global design tokens (colors, fonts, sizes).",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "elementor://dynamic-tags",
            "name": "Dynamic Tags",
            "description": "Registered Elementor dynamic tags and their groups/controls.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "elementor://interactions/schema",
            "name": "Interactions Schema",
            "description": "Elementor V4 native interactions schema (triggers, effects, timing, easing).",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "elementor://breakpoints",
            "name": "Breakpoints",
            "description": "Active Elementor responsive breakpoints and widths.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "elementor://wordpress/best-practices",
            "name": "WordPress & Theme Builder Best Practices",
            "description": "Guidance for Theme Builder singles, display conditions, and dynamic tags.",
            "mimeType": "text/markdown"
        }),
    ]
}

pub fn static_interactions_schema() -> Value {
    json!({
        "type": "array",
        "description": "List of native Elementor V4 interactions attached to an element.",
        "items": {
            "type": "object",
            "required": ["interaction_id", "trigger", "animation"],
            "properties": {
                "interaction_id": { "type": "string" },
                "trigger": {
                    "type": "string",
                    "enum": ["load", "scrollIn", "scrollOut", "scrollOn", "click", "hover"]
                },
                "breakpoints": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "animation": {
                    "type": "object",
                    "required": ["effect", "type", "direction", "timing_config"],
                    "properties": {
                        "effect": { "type": "string", "enum": ["fade", "slide", "scale"] },
                        "type": { "type": "string", "enum": ["in", "out"] },
                        "direction": { "type": "string", "enum": ["", "top", "bottom", "left", "right"] },
                        "timing_config": {
                            "type": "object",
                            "properties": {
                                "duration": { "type": "number", "description": "Milliseconds (e.g. 600)" },
                                "delay": { "type": "number", "description": "Milliseconds (e.g. 0)" }
                            }
                        },
                        "config": {
                            "type": "object",
                            "properties": {
                                "replay": { "type": "boolean" },
                                "easing": { "type": "string", "enum": ["easeInOut", "easeOut", "easeIn", "linear"] },
                                "relativeTo": { "type": "string", "enum": ["viewport", "page"] },
                                "offsetTop": { "type": "number" },
                                "offsetBottom": { "type": "number" }
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn static_wp_best_practices() -> &'static str {
    "# Elementor & WordPress Best Practices\n\n\
    1. **Single Templates vs Duplicated Pages**: When building repeating layouts (e.g. blog posts, projects, products), create one Theme Builder `single` / `single-post` template driven by dynamic tags instead of duplicating static pages.\n\
    2. **Condition Scoping**: Use `include/singular/post` + `exclude/singular/post/<id>` rather than listing dozens of individual post IDs.\n\
    3. **Site Parts Stay Drafts**: `manage_site_parts` creates site parts as drafts; call `publish_document` after reviewing to activate them on the front end.\n\
    4. **Atomic Design Tokens**: Define colors/fonts in `manage_global_variable`, reusable styles in `manage_classes`, and reusable widget groups in `manage_component`.\n"
}

pub async fn fetch_resources_list(wp: &WpClient) -> Value {
    if wp.is_configured()
        && let Ok(resp) = wp.get("mcp-for-page-builders/v1/v4/resources").await
        && resp.get("resources").is_some()
    {
        return resp;
    }
    json!({ "resources": static_resource_catalog() })
}

pub async fn fetch_resource_by_uri(wp: &WpClient, uri: &str) -> Result<Value> {
    if wp.is_configured() {
        let encoded = url_encode(uri);
        if let Ok(resp) = wp
            .get(&format!(
                "mcp-for-page-builders/v1/v4/resource?uri={encoded}"
            ))
            .await
        {
            return Ok(resp);
        }
    }
    match uri {
        "elementor://interactions/schema" => Ok(json!({
            "uri": uri,
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&static_interactions_schema())?,
        })),
        "elementor://wordpress/best-practices" => Ok(json!({
            "uri": uri,
            "mimeType": "text/markdown",
            "text": static_wp_best_practices(),
        })),
        _ => anyhow::bail!(
            "Resource '{uri}' requires an active WordPress connection with the MCP bridge installed."
        ),
    }
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[async_trait]
impl Tool for ListResources {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_resources",
            description: "Returns all available Elementor MCP resource URIs (elementor://global-classes, elementor://global-variables, elementor://dynamic-tags, elementor://interactions/schema, etc.) with descriptions.",
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let resp = fetch_resources_list(wp).await;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}

#[async_trait]
impl Tool for ReadResource {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "read_resource",
            description: "Fetches the content of an Elementor MCP resource by URI (e.g. elementor://global-classes, elementor://global-variables, elementor://dynamic-tags, elementor://interactions/schema).",
            input_schema: json!({
                "type": "object",
                "required": ["uri"],
                "properties": {
                    "uri": { "type": "string", "description": "The resource URI to read, as returned by list_resources." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let uri = str_arg(&args, "uri").ok_or_else(|| anyhow::anyhow!("uri required"))?;
        let resp = fetch_resource_by_uri(wp, &uri).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&resp)?))
    }
}
