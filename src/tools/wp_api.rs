use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct WpApi;
pub struct ListUsers;
pub struct ListComments;
pub struct ListCategories;
pub struct ListTags;
pub struct ListMedia;
pub struct WpSearch;

fn list_tool(
    name: &'static str,
    desc: &'static str,
    endpoint: &'static str,
) -> (ToolDef, &'static str) {
    (
        ToolDef {
            name,
            description: desc,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "per_page": {"type": "integer", "description": "Results per page (max 100)", "default": 10},
                    "page": {"type": "integer", "description": "Page number", "default": 1},
                    "search": {"type": "string", "description": "Search term"}
                }
            }),
        },
        endpoint,
    )
}

async fn run_list(args: &Value, wp: &WpClient, endpoint: &str) -> Result<ToolResult> {
    wp.require_configured()?;
    let per_page = args.get("per_page").and_then(Value::as_i64).unwrap_or(10);
    let page = args.get("page").and_then(Value::as_i64).unwrap_or(1);
    let mut query = json!({"per_page": per_page, "page": page});
    if let Some(s) = args.get("search").and_then(Value::as_str) {
        query["search"] = json!(s);
    }
    let result = wp.request("GET", endpoint, None, Some(&query)).await?;
    Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
}

#[async_trait]
impl Tool for WpApi {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "wp_api",
            description: "Call any WordPress REST API endpoint",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"], "description": "HTTP method"},
                    "endpoint": {"type": "string", "description": "REST endpoint path (e.g. 'wp/v2/users')"},
                    "body": {"type": "object", "description": "Request body (for POST/PUT/PATCH)"},
                    "query": {"type": "object", "description": "Query parameters"}
                },
                "required": ["method", "endpoint"]
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let method = args["method"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("method required"))?;
        let endpoint = args["endpoint"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("endpoint required"))?;
        let body = args.get("body");
        let query = args.get("query");
        reject_known_elementor_write(method, endpoint, body, wp).await?;
        let result = wp.request(method, endpoint, body, query).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}

async fn reject_known_elementor_write(
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
    wp: &WpClient,
) -> Result<()> {
    use reqwest::Url;

    let method = method.to_ascii_uppercase();
    let _base = Url::parse(wp.base_url())?;
    let endpoint_url = Url::parse(&wp.url(endpoint))?;
    let path = endpoint_url.path();
    let normalized = path.trim_matches('/').to_ascii_lowercase();
    if normalized.contains('%') {
        anyhow::bail!(
            "Encoded endpoint paths are rejected; use the checked Elementor tools for document and resource writes"
        );
    }
    let segments: Vec<&str> = normalized.split('/').collect();
    let bridge = normalized.starts_with("wp-json/mcp-for-page-builders/v1/")
        || normalized.starts_with("mcp-for-page-builders/v1/");
    let elementor_native =
        normalized.starts_with("wp-json/elementor/v1/") || normalized.starts_with("elementor/v1/");
    if method != "GET" && bridge {
        anyhow::bail!(
            "Known MCP bridge writes are blocked in wp_api; use apply_elementor_plan, update_page, create_page, or create_template"
        );
    }
    if method != "GET" && elementor_native {
        anyhow::bail!(
            "Elementor native writes are blocked in wp_api; use the checked Elementor tools"
        );
    }

    let core_prefix = if segments.starts_with(&["wp-json", "wp", "v2"]) {
        3
    } else if segments.starts_with(&["wp", "v2"]) {
        2
    } else {
        0
    };
    if method == "DELETE"
        && core_prefix > 0
        && ["pages", "posts", "elementor_library"]
            .contains(&segments.get(core_prefix).copied().unwrap_or(""))
    {
        anyhow::bail!(
            "Direct WordPress post deletion is blocked; use delete_page, delete_post, or delete_template with the current deletion_revision"
        );
    }
    if method == "GET" {
        return Ok(());
    }
    if core_prefix == 0 {
        return Ok(());
    }
    let post_type = segments.get(core_prefix).copied().unwrap_or("");
    if post_type == "elementor_library" {
        anyhow::bail!(
            "Direct Elementor template writes are blocked; use the checked Elementor tools"
        );
    }
    if !["pages", "posts"].contains(&post_type) {
        return Ok(());
    }
    let body_has_elementor = body.is_some_and(contains_protected_elementor_field);
    if body_has_elementor {
        anyhow::bail!(
            "Direct WordPress page/post writes containing Elementor data or conditions are blocked; use the checked Elementor tools"
        );
    }
    let resource_id = segments.get(core_prefix + 1).copied();
    if let Some(id) =
        resource_id.filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
    {
        let lookup = if post_type == "pages" {
            "page"
        } else {
            "post_metadata"
        };
        let state = wp
            .get(&format!("mcp-for-page-builders/v1/resources/{lookup}/{id}"))
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Could not inspect WordPress resource before write; write blocked: {error}"
                )
            })?;
        let value = state
            .get("value")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                anyhow::anyhow!("Invalid resource inspection response; write blocked")
            })?;
        if !state
            .get("revision")
            .and_then(Value::as_str)
            .is_some_and(|revision| !revision.is_empty())
            || !value
                .get("protected_elementor")
                .and_then(Value::as_bool)
                .is_some()
            || !value.get("title").and_then(Value::as_str).is_some()
            || !value.get("status").and_then(Value::as_str).is_some()
            || (lookup == "post_metadata"
                && !value.get("post_type").and_then(Value::as_str).is_some())
        {
            anyhow::bail!("Incomplete resource inspection response; write blocked");
        }
        if value["protected_elementor"] == true
            || value.get("post_type").and_then(Value::as_str) == Some("elementor_library")
        {
            anyhow::bail!(
                "Direct writes to an Elementor document are blocked; use update_page or update_post with the current revision"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod mutation_guard_tests {
    use super::contains_protected_elementor_field;
    use serde_json::json;

    #[test]
    fn protected_fields_are_rejected_recursively_but_ordinary_fields_are_allowed() {
        assert!(contains_protected_elementor_field(
            &json!({"meta":{"_elementor_data":"[]"}})
        ));
        assert!(contains_protected_elementor_field(
            &json!({"meta":{"_elementor_custom_setting":"protected"}})
        ));
        assert!(!contains_protected_elementor_field(
            &json!({"title":"Ordinary post","content":"text","status":"draft"})
        ));
    }
}

fn contains_protected_elementor_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            key.starts_with("_elementor_") || contains_protected_elementor_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_protected_elementor_field),
        _ => false,
    }
}

macro_rules! impl_list_tool {
    ($ty:ident, $endpoint:expr, $name:expr, $desc:expr) => {
        #[async_trait]
        impl Tool for $ty {
            fn def(&self) -> ToolDef {
                list_tool($name, $desc, $endpoint).0
            }
            async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
                run_list(&args, wp, $endpoint).await
            }
        }
    };
}

impl_list_tool!(
    ListUsers,
    "wp/v2/users",
    "list_users",
    "List WordPress users"
);
impl_list_tool!(
    ListComments,
    "wp/v2/comments",
    "list_comments",
    "List WordPress comments"
);
impl_list_tool!(
    ListCategories,
    "wp/v2/categories",
    "list_categories",
    "List WordPress categories"
);
impl_list_tool!(ListTags, "wp/v2/tags", "list_tags", "List WordPress tags");
impl_list_tool!(
    ListMedia,
    "wp/v2/media",
    "list_media",
    "List WordPress media items"
);

#[async_trait]
impl Tool for WpSearch {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "wp_search",
            description: "Search across all WordPress content types",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "search": {"type": "string", "description": "Search term"},
                    "type": {"type": "string", "description": "Content type filter (post, page, etc.)"},
                    "per_page": {"type": "integer", "default": 10}
                },
                "required": ["search"]
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.require_configured()?;
        let search = args["search"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("search required"))?;
        let mut query = json!({"search": search});
        if let Some(t) = args.get("type").and_then(Value::as_str) {
            query["type"] = json!(t);
        }
        if let Some(pp) = args.get("per_page").and_then(Value::as_i64) {
            query["per_page"] = json!(pp);
        }
        let result = wp
            .request("GET", "wp/v2/search", None, Some(&query))
            .await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}
