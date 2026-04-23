use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetSessionState;

#[async_trait]
impl Tool for GetSessionState {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_session_state",
            description: "Get current MCP server session state: active site, PID, uptime, recent operations, and any pending (incomplete) operations from a previous session. Call this at the start of a new session to recover context after a crash or restart.",
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let (pid, uptime_secs, pending, recent) = match &wp.session {
            Some(s) => {
                let uptime = now.saturating_sub(s.started_at);
                (s.pid, uptime, s.pending_ops(), s.recent_ops(5))
            }
            None => (0, 0, vec![], vec![]),
        };

        let pending_json: Vec<Value> = pending.iter().map(|e| json!({
            "id": e.id, "op": e.op, "site": e.site, "subject": e.subject, "ts": e.ts
        })).collect();

        let recent_json: Vec<Value> = recent.iter().map(|e| json!({
            "op": e.op, "site": e.site, "subject": e.subject, "ts": e.ts
        })).collect();

        let state = json!({
            "pid": pid,
            "uptime_secs": uptime_secs,
            "active_site": if wp.is_configured() { wp.base_url() } else { "(none)" },
            "pending_ops": pending_json,
            "recent_ops": recent_json,
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&state)?))
    }
}
