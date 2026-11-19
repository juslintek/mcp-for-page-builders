use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::BRIDGE_SLUG;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

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
            return Ok(ToolResult::text(format!(
                "Bridge already installed (v{ver})"
            )));
        }

        if wp
            .post(
                "wp/v2/plugins",
                &json!({"slug": BRIDGE_SLUG, "status": "active"}),
            )
            .await
            .is_ok()
        {
            return Ok(ToolResult::text(format!(
                "Bridge installed from wordpress.org ({BRIDGE_SLUG})"
            )));
        }

        if activate_bridge_via_rest(wp).await.is_ok()
            && wp.get("mcp-for-page-builders/v1/status").await.is_ok()
        {
            return Ok(ToolResult::text(
                "Bridge was inactive — activated successfully.".to_string(),
            ));
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

const BRIDGE_MU_PHP: &str = include_str!("../../../assets/mcp-bridge.php");

async fn activate_bridge_via_rest(wp: &WpClient) -> Result<()> {
    wp.request(
        "PUT",
        "wp/v2/plugins/mcp-bridge-for-page-builders/mcp-bridge-for-page-builders",
        Some(&json!({"status": "active"})),
        None,
    )
    .await?;
    Ok(())
}

/// Bootstrap bridge by: install Code Snippets → create snippet that writes bridge plugin →
/// activate bridge → delete snippet → remove Code Snippets. No passwords or cookies needed.
async fn bootstrap_via_snippets(wp: &WpClient) -> Result<String> {
    use tracing::info;

    // 1. Install Code Snippets plugin
    info!("Installing code-snippets plugin...");
    wp.post(
        "wp/v2/plugins",
        &json!({"slug": "code-snippets", "status": "active"}),
    )
    .await
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
    let snippet = wp
        .post(
            "code-snippets/v1/snippets",
            &json!({
                "name": "MCP Bridge Installer (temporary)",
                "code": snippet_code,
                "scope": "global",
                "active": true,
            }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Cannot create snippet: {e:#}"))?;

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
        let _ = wp
            .request(
                "DELETE",
                &format!("code-snippets/v1/snippets/{snippet_id}"),
                None,
                None,
            )
            .await;
    }
    let _ = wp
        .request(
            "PUT",
            "wp/v2/plugins/code-snippets/code-snippets",
            Some(&json!({"status": "inactive"})),
            None,
        )
        .await;
    let _ = wp
        .request(
            "DELETE",
            "wp/v2/plugins/code-snippets/code-snippets",
            None,
            None,
        )
        .await;

    if verified {
        Ok("Bridge installed via Code Snippets bootstrap (snippets plugin removed).".into())
    } else if activated {
        Ok("Bridge plugin activated but status endpoint not responding yet. Try again in a moment.".into())
    } else {
        anyhow::bail!("Snippet executed but bridge plugin could not be activated.")
    }
}
