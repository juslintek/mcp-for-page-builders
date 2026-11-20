use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::Tool;
use crate::args::{str_arg, u64_arg};
use crate::cache::{CacheScope, clear_related_caches, detect_caches};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct ClearCache;

#[async_trait]
impl Tool for ClearCache {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "clear_cache",
            description: "Detect and clear Elementor CSS cache and any active WordPress/CDN/proxy caches (LiteSpeed, WP Rocket, W3TC, WP Super Cache, SG Optimizer, Object Cache, etc.).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["clear", "detect"],
                        "default": "clear",
                        "description": "Use 'detect' to inspect active cache layers without clearing, or 'clear' to purge."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["all", "post", "global"],
                        "description": "Cache scope to clear. Defaults to 'post' when post_id is provided, otherwise 'all'."
                    },
                    "post_id": {
                        "type": "integer",
                        "description": "Optional post/page/template ID for targeted post cache clearing."
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let action = str_arg(&args, "action").unwrap_or_else(|| "clear".to_string());
        match action.as_str() {
            "detect" => {
                let report = detect_caches(wp).await;
                Ok(ToolResult::text(serde_json::to_string_pretty(&report)?))
            }
            "clear" => {
                let post_id = u64_arg(&args, "post_id");
                let scope_str = str_arg(&args, "scope");
                let scope = match scope_str.as_deref() {
                    Some("post") => {
                        let id = post_id.ok_or_else(|| {
                            anyhow::anyhow!("post_id is required when scope is 'post'")
                        })?;
                        CacheScope::Post(id)
                    }
                    Some("global") => CacheScope::Global,
                    Some("all") => CacheScope::All,
                    Some(other) => bail!("Unknown cache scope '{other}'"),
                    None => match post_id {
                        Some(id) => CacheScope::Post(id),
                        None => CacheScope::All,
                    },
                };
                let report = clear_related_caches(wp, scope).await;
                Ok(ToolResult::text(serde_json::to_string_pretty(&report)?))
            }
            other => bail!("Unknown cache action '{other}'; expected 'clear' or 'detect'"),
        }
    }
}
