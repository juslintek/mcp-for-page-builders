mod args;
mod types;
mod util;
mod mcp;
mod wp;
mod elementor;
mod tools;
mod setup;
pub mod cdp;

use anyhow::Result;
use serde_json::{json, Value};
use tracing::info;

use crate::mcp::{Response, Stdio};
use crate::tools::Tool;
use crate::wp::WpClient;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Subcommand: setup
    if args.get(1).map(std::string::String::as_str) == Some("setup") {
        let url = args.get(2).ok_or_else(|| anyhow::anyhow!(
            "Usage: elementor-mcp setup <wordpress-url>\n  Example: elementor-mcp setup https://my-site.com"
        ))?;
        return setup::run(url).await;
    }

    // MCP server mode
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "elementor_mcp=info".parse().unwrap()),
        )
        .init();

    // Load site store
    let mut site_store = crate::wp::SiteStore::load();

    // If WP_URL env var is set, add that site to store
    if let Ok(url) = std::env::var("WP_URL") {
        let user = env_or("WP_APP_USER", "admin");
        let pass = std::env::var("WP_APP_PASSWORD").unwrap_or_default();
        let url = url.trim_end_matches('/').to_string();
        site_store.add_site(crate::wp::SiteCredentials { url: url.clone(), user, app_password: pass });
        site_store.active = Some(url);
    }

    let store = std::sync::Arc::new(tokio::sync::RwLock::new(site_store));

    let wp = {
        let s = store.read().await;
        if let Some(creds) = s.get_active() {
            WpClient::from_creds(creds).with_store(store.clone())
        } else {
            eprintln!("No WP_URL set — starting in CDP-only mode. WordPress tools will prompt for setup.");
            WpClient::unconfigured().with_store(store.clone())
        }
    };

    let tools = tools::all_tools();
    let mut stdio = Stdio::new();
    let mode = if wp.is_configured() { wp.base_url().to_string() } else { "CDP-only (no WordPress)".into() };

    info!("elementor-mcp started ({} tools) → {}", tools.len(), mode);

    loop {
        let Some(req) = stdio.read_request().await? else {
            break;
        };
        // Notifications (no id) must not receive a response per JSON-RPC 2.0
        if req.id.is_none() {
            continue;
        }
        let resp = handle(&req.method, &req.params, req.id.clone(), &tools, &wp).await;
        stdio.write_response(&resp).await?;
    }

    Ok(())
}

async fn handle(
    method: &str, params: &Value, id: Option<Value>,
    tools: &[Box<dyn Tool>], wp: &WpClient,
) -> Response {
    match method {
        "initialize" => Response::ok(id, json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "elementor-mcp", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => {
            let defs: Vec<_> = tools.iter().map(|t| {
                let d = t.def();
                json!({"name": d.name, "description": d.description, "inputSchema": d.input_schema})
            }).collect();
            Response::ok(id, json!({"tools": defs}))
        }
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match tools.iter().find(|t| t.def().name == name) {
                Some(t) => {
                    info!("Calling tool: {name}");
                    match t.run(args, wp).await {
                        Ok(result) => Response::ok(id, serde_json::to_value(result).unwrap()),
                        Err(e) => {
                            let result = mcp::ToolResult::error(format!("{e:#}"));
                            Response::ok(id, serde_json::to_value(result).unwrap())
                        }
                    }
                }
                None => Response::err(id, -32601, format!("Unknown tool: {name}")),
            }
        }
        _ => Response::err(id, -32601, format!("Unknown method: {method}")),
    }
}
