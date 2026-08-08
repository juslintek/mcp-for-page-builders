use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;
use super::engine::ShadowEngine;
use super::portal::{render_portal_html, PortalBadge, PortalInfo};

/// MCP Tool: Spawn a new isolated Shadow Realm session
pub struct ShadowSpawn;

#[async_trait]
impl Tool for ShadowSpawn {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shadow_spawn",
            description: "Spawn a new isolated background Shadow Realm session for an application or target URL. Runs completely in memory without taking over mouse hardware or focus.",
            input_schema: json!({
                "type": "object",
                "required": ["app_name", "target_url"],
                "properties": {
                    "app_name": { "type": "string", "description": "Name of the target desktop application (e.g. Teams, Slack, Chrome, CustomApp)" },
                    "target_url": { "type": "string", "description": "Target URL or endpoint to open inside the shadow realm" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let app_name = str_arg(&args, "app_name").ok_or_else(|| anyhow::anyhow!("app_name required"))?;
        let target_url = str_arg(&args, "target_url").ok_or_else(|| anyhow::anyhow!("target_url required"))?;

        let engine = ShadowEngine::global();
        let branch = engine.spawn(&app_name, &target_url).await?;

        let res = json!({
            "status": "success",
            "shadow_id": branch.id,
            "app_name": branch.app_name,
            "target_url": branch.target_url,
            "profile_path": branch.profile_path,
            "display_id": branch.display_id,
            "stream_url": branch.stream_url,
            "portal_hint": "Click 🔮 Portal Badge on your desktop to monitor live video feed"
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&res)?))
    }
}

/// MCP Tool: Fork an existing Shadow Realm into a parallel dimension
pub struct ShadowFork;

#[async_trait]
impl Tool for ShadowFork {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shadow_fork",
            description: "Fork an active Shadow Realm instance into a new parallel dimension/branch to test alternative strategies simultaneously without interrupting the primary branch.",
            input_schema: json!({
                "type": "object",
                "required": ["parent_shadow_id"],
                "properties": {
                    "parent_shadow_id": { "type": "string", "description": "ID of the parent shadow realm instance to fork" },
                    "branch_name": { "type": "string", "description": "Optional descriptive name for this parallel branch (e.g. 'strategy-b-variant')" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let parent_id = str_arg(&args, "parent_shadow_id").ok_or_else(|| anyhow::anyhow!("parent_shadow_id required"))?;
        let branch_name = str_arg(&args, "branch_name");

        let engine = ShadowEngine::global();
        let branch = engine.fork(&parent_id, branch_name.as_deref()).await?;

        let res = json!({
            "status": "success",
            "forked_shadow_id": branch.id,
            "parent_shadow_id": branch.parent_id,
            "branch_depth": branch.branch_depth,
            "stream_url": branch.stream_url,
            "message": "Parallel dimension successfully spawned and executing in background."
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&res)?))
    }
}

/// MCP Tool: List active Shadow Realm dimensions in the DAG
pub struct ShadowList;

#[async_trait]
impl Tool for ShadowList {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shadow_list",
            description: "List all active Shadow Realm instances, their parallel branch DAG hierarchy, status, and live stream URLs.",
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn run(&self, _args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let engine = ShadowEngine::global();
        let branches = engine.list().await;

        let res = json!({
            "total_active_shadows": branches.len(),
            "branches": branches
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&res)?))
    }
}

/// MCP Tool: Promote a shadow realm's execution results back to main target
pub struct ShadowPromote;

#[async_trait]
impl Tool for ShadowPromote {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shadow_promote",
            description: "Promote/merge a shadow realm's execution results back to the primary target session.",
            input_schema: json!({
                "type": "object",
                "required": ["shadow_id"],
                "properties": {
                    "shadow_id": { "type": "string", "description": "ID of the shadow realm instance to promote" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let shadow_id = str_arg(&args, "shadow_id").ok_or_else(|| anyhow::anyhow!("shadow_id required"))?;

        let engine = ShadowEngine::global();
        let branch = engine.promote(&shadow_id).await?;

        let res = json!({
            "status": "promoted",
            "promoted_shadow_id": branch.id,
            "message": "Shadow realm state promoted successfully to primary session."
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&res)?))
    }
}

/// MCP Tool: Get Portal UI Badge status and live streaming URL
pub struct ShadowPortalInfo;

#[async_trait]
impl Tool for ShadowPortalInfo {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "shadow_portal_info",
            description: "Get Portal UI indicator status, badge state, live WebRTC/WebSocket stream endpoints, and rendered HTML for the Portal viewer window.",
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn run(&self, _args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let engine = ShadowEngine::global();
        let branches = engine.list().await;

        let badge = PortalBadge {
            label: format!("🔮 Portal: {} Active Shadows", branches.len()),
            active_shadow_count: branches.len(),
            primary_branch_id: branches.first().map(|b| b.id.clone()),
            portal_url: "http://localhost:9100/portal".to_string(),
        };

        let portal_html = render_portal_html(&branches);

        let info = PortalInfo {
            title: "🔮 Shadow Realm Multiverse Portal".to_string(),
            active_branches: branches,
            badge,
            portal_html,
        };

        Ok(ToolResult::text(serde_json::to_string_pretty(&info)?))
    }
}
