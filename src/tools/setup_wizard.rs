use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct SetupWizard;

#[async_trait]
impl Tool for SetupWizard {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "setup_wizard",
            description: "Configure WordPress connection. Call this when WordPress tools return 'not configured' errors.\n\nReturns setup options with step-by-step instructions. CDP-only tools (screenshot, visual_compare, inspect_page, etc.) work without WordPress.",
            input_schema: json!({"type":"object","properties":{}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        if wp.is_configured() {
            return Ok(ToolResult::text(format!(
                "✓ WordPress is already configured: {}\n\nTo change, update the env vars in your MCP config.",
                wp.base_url()
            )));
        }

        Ok(ToolResult::text(
r#"WordPress is not configured. Choose a setup option:

━━━ Option A: Edit MCP config (recommended) ━━━
Add env vars to your MCP client config (e.g. .kiro/settings/mcp.json):

  {
    "mcpServers": {
      "mcp-for-page-builders": {
        "command": "/path/to/mcp-for-page-builders",
        "env": {
          "WP_URL": "https://your-site.com",
          "WP_APP_USER": "admin",
          "WP_APP_PASSWORD": "xxxx xxxx xxxx xxxx xxxx xxxx",
          "WP_TLS_INSECURE": "1"
        }
      }
    }
  }

Then restart your chat session to reload the MCP server.

━━━ Option B: CLI setup ━━━
Run from terminal:  mcp-for-page-builders setup https://your-site.com

━━━ Option C: Create an Application Password ━━━
WordPress Admin → Users → Profile → Application Passwords → Add New
Or via WP-CLI:  wp user application-password create admin "mcp" --porcelain

━━━ What works without WordPress ━━━
These tools work right now (CDP-only, no WordPress needed):
  • screenshot — capture any URL
  • visual_compare — side-by-side comparison of any two URLs
  • visual_diff — element-by-element comparison
  • inspect_page — DOM inspection
  • extract_styles — computed CSS extraction
  • clone_element — DOM to Elementor JSON
  • css_to_elementor — CSS to Elementor settings
  • list_widgets / get_widget_schema / validate_element — schema tools"#.to_string()))
    }
}
