use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::tool_def::ToolDef;
use super::tool_result::ToolResult;
use crate::wp::WpClient;

#[async_trait]
pub trait Tool: Send + Sync {
    fn def(&self) -> ToolDef;
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult>;
}
