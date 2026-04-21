use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct WpApi;
pub struct ListUsers;
pub struct ListComments;
pub struct ListCategories;
pub struct ListTags;
pub struct ListMedia;
pub struct WpSearch;

fn list_tool(name: &'static str, desc: &'static str, endpoint: &'static str) -> (ToolDef, &'static str) {
    (ToolDef {
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
    }, endpoint)
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
        let method = args["method"].as_str().ok_or_else(|| anyhow::anyhow!("method required"))?;
        let endpoint = args["endpoint"].as_str().ok_or_else(|| anyhow::anyhow!("endpoint required"))?;
        let body = args.get("body");
        let query = args.get("query");
        let result = wp.request(method, endpoint, body, query).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}

macro_rules! impl_list_tool {
    ($ty:ident, $endpoint:expr, $name:expr, $desc:expr) => {
        #[async_trait]
        impl Tool for $ty {
            fn def(&self) -> ToolDef { list_tool($name, $desc, $endpoint).0 }
            async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
                run_list(&args, wp, $endpoint).await
            }
        }
    };
}

impl_list_tool!(ListUsers, "wp/v2/users", "list_users", "List WordPress users");
impl_list_tool!(ListComments, "wp/v2/comments", "list_comments", "List WordPress comments");
impl_list_tool!(ListCategories, "wp/v2/categories", "list_categories", "List WordPress categories");
impl_list_tool!(ListTags, "wp/v2/tags", "list_tags", "List WordPress tags");
impl_list_tool!(ListMedia, "wp/v2/media", "list_media", "List WordPress media items");

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
        let search = args["search"].as_str().ok_or_else(|| anyhow::anyhow!("search required"))?;
        let mut query = json!({"search": search});
        if let Some(t) = args.get("type").and_then(Value::as_str) {
            query["type"] = json!(t);
        }
        if let Some(pp) = args.get("per_page").and_then(Value::as_i64) {
            query["per_page"] = json!(pp);
        }
        let result = wp.request("GET", "wp/v2/search", None, Some(&query)).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}
