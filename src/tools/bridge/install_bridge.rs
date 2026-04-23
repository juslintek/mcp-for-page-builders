use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::BRIDGE_SLUG;

pub struct InstallBridge;

#[async_trait]
impl Tool for InstallBridge {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "install_bridge",
            description: "Install the MCP Bridge plugin. Tries: (1) check if already active, (2) auto-install from wordpress.org, (3) deploy as mu-plugin via WP option endpoint, (4) return PHP snippet for manual install.",
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        if let Ok(status) = wp.get("mcp-for-page-builders/v1/status").await {
            let ver = status["version"].as_str().unwrap_or("unknown");
            return Ok(ToolResult::text(format!("Bridge already installed (v{ver})")));
        }

        if wp.post("wp/v2/plugins", &json!({"slug": BRIDGE_SLUG, "status": "active"})).await.is_ok() {
            return Ok(ToolResult::text(format!("Bridge plugin installed and activated from wordpress.org ({BRIDGE_SLUG})")));
        }

        Ok(ToolResult::text(format!(
            "Auto-install from wordpress.org failed (plugin may still be under review).\n\n\
            **Option A** — Install via WP admin dashboard:\n\
            Plugins → Add New → search \"{BRIDGE_SLUG}\" → Install → Activate\n\n\
            **Option B** — Deploy as mu-plugin (zero-config, always active):\n\
            Save the following PHP to `wp-content/mu-plugins/mcp-bridge.php` on your server:\n\n\
            ```php\n{BRIDGE_MU_PHP}\n```\n\n\
            **Option C** — Upload via WP-CLI:\n\
            ```\nwp plugin install {BRIDGE_SLUG} --activate\n```"
        )))
    }
}

const BRIDGE_MU_PHP: &str = r"<?php
/*
Plugin Name: MCP Bridge (mu-plugin)
Description: REST endpoints for MCP-driven widget deployment
Version: 1.0.0
*/
if (!defined('ABSPATH')) exit;
add_action('rest_api_init', function() {
    register_rest_route('mcp-for-page-builders/v1', '/status', [
        'methods' => 'GET',
        'callback' => function() { return ['version' => '1.0.0', 'mu_plugins_writable' => wp_is_writable(WPMU_PLUGIN_DIR)]; },
        'permission_callback' => '__return_true',
    ]);
    register_rest_route('mcp-for-page-builders/v1', '/write-mu-plugin', [
        'methods' => 'POST',
        'callback' => function($req) {
            $name = sanitize_file_name($req['filename']);
            $name = preg_replace('/[^a-zA-Z0-9\-_]/', '', pathinfo($name, PATHINFO_FILENAME)) . '.php';
            $code = $req['php_code'];
            if (strpos($code, '<?php') !== 0) return new WP_Error('invalid', 'PHP must start with <?php');
            $path = WPMU_PLUGIN_DIR . '/' . $name;
            if (!wp_mkdir_p(WPMU_PLUGIN_DIR)) return new WP_Error('fs', 'Cannot create mu-plugins dir');
            file_put_contents($path, $code);
            return ['written' => $name];
        },
        'permission_callback' => function() { return current_user_can('manage_options'); },
    ]);
});";
