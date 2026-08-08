use serde::{Deserialize, Serialize};
use super::engine::ShadowBranch;

/// Portal Badge Indicator state attached to target desktop applications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalBadge {
    pub label: String,
    pub active_shadow_count: usize,
    pub primary_branch_id: Option<String>,
    pub portal_url: String,
}

/// Metadata and HTML renderer for the Portal Preview Window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalInfo {
    pub title: String,
    pub active_branches: Vec<ShadowBranch>,
    pub badge: PortalBadge,
    pub portal_html: String,
}

use std::fmt::Write;

pub fn render_portal_html(branches: &[ShadowBranch]) -> String {
    let mut cards = String::new();
    for b in branches {
        let _ = write!(
            cards,
            r#"<div class="card">
                <div class="card-header">
                    <span class="badge">{}</span>
                    <h3>{}</h3>
                </div>
                <p>Target: <code>{}</code></p>
                <p>Display: <code>{}</code> | Depth: {}</p>
                <div class="stream-container">
                    <div class="stream-placeholder">Live Feed: {}</div>
                </div>
                <div class="card-actions">
                    <button onclick="forkShadow('{}')">🌿 Fork Parallel Shadow</button>
                    <button onclick="promoteShadow('{}')">⚡ Promote to Main</button>
                </div>
            </div>"#,
            b.id, b.app_name, b.target_url, b.display_id, b.branch_depth, b.stream_url, b.id, b.id
        );
    }

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>🔮 Shadow Realm Multiverse Portal</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: system-ui, -apple-system, sans-serif; background: #0b0c10; color: #c5c6c7; padding: 20px; }}
        header {{ display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid #1f2833; padding-bottom: 15px; margin-bottom: 20px; }}
        h1 {{ font-size: 20px; color: #66fcf1; display: flex; align-items: center; gap: 8px; }}
        .grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(400px, 1fr)); gap: 20px; }}
        .card {{ background: #1f2833; border: 1px solid #45a29e; border-radius: 8px; padding: 16px; display: flex; flex-direction: column; gap: 10px; }}
        .card-header {{ display: flex; justify-content: space-between; align-items: center; }}
        .badge {{ background: #45a29e; color: #0b0c10; font-weight: 700; font-size: 11px; padding: 4px 8px; border-radius: 4px; }}
        .stream-container {{ background: #000; border-radius: 6px; height: 240px; display: flex; align-items: center; justify-content: center; color: #66fcf1; font-family: monospace; font-size: 12px; }}
        .card-actions {{ display: flex; gap: 10px; margin-top: 10px; }}
        button {{ flex: 1; background: #66fcf1; color: #0b0c10; border: none; padding: 8px 12px; border-radius: 6px; font-weight: 600; cursor: pointer; transition: opacity 0.2s; }}
        button:hover {{ opacity: 0.85; }}
    </style>
</head>
<body>
    <header>
        <h1>🔮 Multiverse Shadow Realm Portal</h1>
        <div>Active Dimensions: {}</div>
    </header>
    <div class="grid">
        {}
    </div>
</body>
</html>"#,
        branches.len(),
        if cards.is_empty() {
            "<p>No active shadow realms running.</p>".to_string()
        } else {
            cards
        }
    )
}
