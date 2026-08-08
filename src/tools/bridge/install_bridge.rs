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
            description: "Install the MCP Bridge plugin. Tries: (1) check if already active (also satisfied by juslintek-cache-manager plugin), (2) auto-install from wordpress.org, (3) deploy as mu-plugin via Code Snippets bootstrap, (4) return PHP snippet for manual install.",
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        if let Ok(status) = wp.get("mcp-for-page-builders/v1/status").await {
            let ver = status["version"].as_str().unwrap_or("unknown");
            return Ok(ToolResult::text(format!("Bridge already installed (v{ver})")));
        }

        if wp.post("wp/v2/plugins", &json!({"slug": BRIDGE_SLUG, "status": "active"})).await.is_ok() {
            return Ok(ToolResult::text(format!("Bridge installed from wordpress.org ({BRIDGE_SLUG})")));
        }

        if activate_bridge_via_rest(wp).await.is_ok()
            && wp.get("mcp-for-page-builders/v1/status").await.is_ok()
        {
            return Ok(ToolResult::text("Bridge was inactive — activated successfully.".to_string()));
        }

        // Bootstrap via Code Snippets: install snippets plugin, create snippet that
        // writes bridge plugin file, activate bridge, then remove snippets plugin.
        match bootstrap_via_snippets(wp).await {
            Ok(msg) => return Ok(ToolResult::text(msg)),
            Err(e) => {
                tracing::warn!("Snippets bootstrap failed: {e:#}");
            }
        }

        Ok(ToolResult::text(format!(
            "Auto-install failed.\n\n\
            Save this PHP to `wp-content/mu-plugins/mcp-bridge.php`:\n\n\
            ```php\n{BRIDGE_MU_PHP}\n```"
        )))
    }
}

const BRIDGE_MU_PHP: &str = r"<?php
/*
Plugin Name: MCP Bridge (mu-plugin)
Description: REST endpoints for MCP-driven file deployment and option management
Version: 1.1.0
*/
if (!defined('ABSPATH')) exit;
add_action('rest_api_init', function() {
    $ns = 'mcp-for-page-builders/v1';
    $admin = function() { return current_user_can('manage_options'); };

    register_rest_route($ns, '/status', [
        'methods' => 'GET',
        'callback' => function() {
            $theme = wp_get_theme();
            return [
                'version' => '1.1.0',
                'mu_plugins_writable' => wp_is_writable(WPMU_PLUGIN_DIR),
                'theme' => $theme->get_stylesheet(),
                'parent_theme' => $theme->get_template(),
                'theme_dir' => $theme->get_stylesheet_directory(),
            ];
        },
        'permission_callback' => '__return_true',
    ]);

    register_rest_route($ns, '/write-mu-plugin', [
        'methods' => 'POST',
        'callback' => function($req) {
            $name = sanitize_file_name($req['filename']);
            $name = preg_replace('/[^a-zA-Z0-9\-_]/', '', pathinfo($name, PATHINFO_FILENAME)) . '.php';
            $code = $req['php_code'];
            if (strpos($code, '<?php') !== 0) return new WP_Error('invalid', 'PHP must start with <?php');
            $path = WPMU_PLUGIN_DIR . '/' . $name;
            if (!wp_mkdir_p(WPMU_PLUGIN_DIR)) return new WP_Error('fs', 'Cannot create mu-plugins dir');
            file_put_contents($path, $code);
            return ['written' => $name, 'path' => $path];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/write-theme-file', [
        'methods' => 'POST',
        'callback' => function($req) {
            $file = ltrim($req['path'], '/');
            if (strpos($file, '..') !== false) return new WP_Error('invalid', 'Path traversal not allowed');
            $theme_dir = get_stylesheet_directory();
            $full = $theme_dir . '/' . $file;
            $dir = dirname($full);
            if (!wp_mkdir_p($dir)) return new WP_Error('fs', 'Cannot create directory: ' . $dir);
            file_put_contents($full, $req['content']);
            return ['written' => $file, 'path' => $full, 'theme' => get_stylesheet()];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/read-theme-file', [
        'methods' => 'GET',
        'callback' => function($req) {
            $file = ltrim($req['path'], '/');
            if (strpos($file, '..') !== false) return new WP_Error('invalid', 'Path traversal not allowed');
            $full = get_stylesheet_directory() . '/' . $file;
            if (!file_exists($full)) return new WP_Error('not_found', 'File not found: ' . $file, ['status' => 404]);
            return ['path' => $file, 'content' => file_get_contents($full)];
        },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/option/(?P<name>[a-zA-Z0-9_\-]+)', [
        'methods' => 'GET',
        'callback' => function($req) { return rest_ensure_response(get_option($req['name'])); },
        'permission_callback' => $admin,
    ]);

    register_rest_route($ns, '/option/(?P<name>[a-zA-Z0-9_\-]+)', [
        'methods' => 'POST',
        'callback' => function($req) {
            $val = $req->get_json_params()['value'] ?? null;
            update_option($req['name'], $val);
            return ['updated' => $req['name']];
        },
        'permission_callback' => $admin,
    ]);
});";

async fn activate_bridge_via_rest(wp: &WpClient) -> Result<()> {
    wp.request("PUT", "wp/v2/plugins/mcp-bridge-for-page-builders/mcp-bridge-for-page-builders",
        Some(&json!({"status": "active"})), None).await?;
    Ok(())
}

/// Bootstrap bridge by: install Code Snippets → create snippet that writes bridge plugin →
/// activate bridge → delete snippet → remove Code Snippets. No passwords or cookies needed.
async fn bootstrap_via_snippets(wp: &WpClient) -> Result<String> {
    use tracing::info;

    // 1. Install Code Snippets plugin
    info!("Installing code-snippets plugin...");
    wp.post("wp/v2/plugins", &json!({"slug": "code-snippets", "status": "active"})).await
        .map_err(|e| anyhow::anyhow!("Cannot install code-snippets: {e:#}"))?;

    // 2. Create snippet that writes bridge plugin file
    let snippet_code = format!(
        "$dir = WP_PLUGIN_DIR . '/mcp-bridge-for-page-builders';\n\
         if (!file_exists($dir)) {{ wp_mkdir_p($dir); }}\n\
         $php = <<<'BRIDGEPHP'\n\
         {BRIDGE_MU_PHP}\n\
         BRIDGEPHP;\n\
         file_put_contents($dir . '/mcp-bridge-for-page-builders.php', $php);"
    );

    info!("Creating installer snippet...");
    let snippet = wp.post("code-snippets/v1/snippets", &json!({
        "name": "MCP Bridge Installer (temporary)",
        "code": snippet_code,
        "scope": "global",
        "active": true,
    })).await.map_err(|e| anyhow::anyhow!("Cannot create snippet: {e:#}"))?;

    let snippet_id = snippet["id"].as_u64().unwrap_or(0);

    // 3. Trigger snippet execution by hitting an authenticated endpoint
    info!("Triggering snippet execution...");
    let _ = wp.get("wp/v2/plugins").await;

    // 4. Activate bridge plugin
    info!("Activating bridge plugin...");
    let activated = activate_bridge_via_rest(wp).await.is_ok();

    // 5. Verify
    let verified = wp.get("mcp-for-page-builders/v1/status").await.is_ok();

    // 6. Cleanup: delete snippet, deactivate + delete Code Snippets
    info!("Cleaning up...");
    if snippet_id > 0 {
        let _ = wp.request("DELETE", &format!("code-snippets/v1/snippets/{snippet_id}"), None, None).await;
    }
    let _ = wp.request("PUT", "wp/v2/plugins/code-snippets/code-snippets",
        Some(&json!({"status": "inactive"})), None).await;
    let _ = wp.request("DELETE", "wp/v2/plugins/code-snippets/code-snippets", None, None).await;

    if verified {
        Ok("Bridge installed via Code Snippets bootstrap (snippets plugin removed).".into())
    } else if activated {
        Ok("Bridge plugin activated but status endpoint not responding yet. Try again in a moment.".into())
    } else {
        anyhow::bail!("Snippet executed but bridge plugin could not be activated.")
    }
}
