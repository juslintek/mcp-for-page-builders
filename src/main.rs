#![allow(
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::return_self_not_must_use,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::type_complexity
)]

mod args;
mod types;
mod util;
mod mcp;
mod wp;
mod elementor;
mod tools;
mod setup;
mod session;
mod logging;
pub mod cdp;
pub mod shadow_realm;

use anyhow::Result;
use futures::FutureExt;
use serde_json::{json, Value};
use std::panic::AssertUnwindSafe;
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
            "Usage: mcp-for-page-builders setup <wordpress-url>\n  Example: mcp-for-page-builders setup https://my-site.com"
        ))?;
        return setup::run(url).await;
    }

    // MCP server mode
    // Keep the guard alive for the whole process — dropping it stops the
    // background file-log writer thread.
    let _log_guard = logging::init()?;

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

    // Acquire session (orphan detection + journal)
    let session = match session::Session::acquire() {
        Ok(s) => {
            let pending = s.pending_ops();
            if !pending.is_empty() {
                eprintln!("⚠ {} pending op(s) from previous session — call get_session_state for details", pending.len());
            }
            Some(std::sync::Arc::new(s))
        }
        Err(e) => { eprintln!("Session acquire failed (non-fatal): {e}"); None }
    };

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<crate::wp::ServerNotification>();

    let mut wp = {
        let s = store.read().await;
        let client = if let Some(creds) = s.get_active() {
            WpClient::from_creds(creds).with_store(store.clone())
        } else {
            eprintln!("No active site — starting in CDP-only mode. WordPress tools will prompt for setup.");
            WpClient::unconfigured().with_store(store.clone())
        };
        let client = client.with_notifier(notify_tx);
        if let Some(sess) = session { client.with_session(sess) } else { client }
    };

    let tools = tools::all_tools();
    let mut stdio = Stdio::new();
    let mode = if wp.is_configured() { wp.base_url().to_string() } else { "CDP-only (no WordPress)".into() };

    info!("mcp-for-page-builders started ({} tools) → {}", tools.len(), mode);

    loop {
        let req = match stdio.read_request().await {
            Ok(Some(req)) => req,
            Ok(None) => break, // true EOF — client closed the pipe
            Err(e) => {
                // A truly malformed line is now handled inside read_request
                // itself (it logs and continues). Reaching here means an
                // I/O-level error on stdin, which is unrecoverable — log it
                // durably before exiting so there's evidence in the file log.
                tracing::error!("Fatal stdin read error: {e:#}");
                break;
            }
        };
        // Notifications (no id) must not receive a response per JSON-RPC 2.0
        if req.id.is_none() {
            continue;
        }

        // Catch panics per-request: a single bad tool call (e.g. a CDP
        // task panicking) must not take down the whole server process,
        // which the MCP client would otherwise observe as "the connection
        // closed" with no diagnosable cause.
        let id_for_panic = req.id.clone();
        let resp = match AssertUnwindSafe(handle(&req.method, &req.params, req.id.clone(), &tools, &wp))
            .catch_unwind()
            .await
        {
            Ok(resp) => resp,
            Err(panic_payload) => {
                let msg = panic_payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                tracing::error!("Tool call panicked (recovered, connection stays alive): {msg}");
                Response::err(id_for_panic, -32603, "Internal error (recovered)".to_string())
            }
        };

        if let Err(e) = stdio.write_response(&resp).await {
            // If we can't write to stdout at all, the pipe is genuinely
            // gone (client exited) — log it and exit cleanly rather than
            // looping forever on a broken pipe.
            tracing::error!("Fatal stdout write error, exiting: {e:#}");
            break;
        }

        // Drain notification channel — handle reconfigure/tool-change requests from tools
        while let Ok(notif) = notify_rx.try_recv() {
            match notif {
                crate::wp::ServerNotification::Reconfigure => {
                    let s = store.read().await;
                    if let Some(creds) = s.get_active() {
                        info!("Reconfiguring WpClient → {}", creds.url);
                        wp.reconfigure(creds);
                    }
                }
                crate::wp::ServerNotification::ToolsChanged => {
                    info!("Sending notifications/tools/list_changed");
                    let _ = stdio.write_notification("notifications/tools/list_changed", json!(null)).await;
                }
            }
        }
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
            "capabilities": { "tools": { "listChanged": true } },
            "serverInfo": { "name": "mcp-for-page-builders", "version": env!("CARGO_PKG_VERSION") }
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
                        Ok(result) => match serde_json::to_value(result) {
                            Ok(v) => Response::ok(id, v),
                            Err(e) => Response::err(id, -32603, format!("Serialization error: {e}")),
                        },
                        Err(e) => {
                            let msg = format!("{e:#}");
                            let result = if let Some(root) = crate::util::dev_project_root() {
                                mcp::ToolResult::error(format!(
                                    "{msg}\n\n\
                                    ---\n\
                                    🔧 **Self-improvement hint** (dev mode detected):\n\
                                    This MCP server is running from a local build at `{root}`.\n\
                                    The tool `{name}` failed with the error above.\n\
                                    To fix: inspect the source in `{root}/src/tools/`, fix the bug, then rebuild:\n\
                                    ```\ncd {root} && cargo build --release\n```\n\
                                    The server will pick up changes on next restart.",
                                    root = root.display()
                                ))
                            } else {
                                mcp::ToolResult::error(msg)
                            };
                            match serde_json::to_value(result) {
                                Ok(v) => Response::ok(id, v),
                                Err(ser_err) => Response::err(id, -32603, format!("Serialization error: {ser_err}")),
                            }
                        }
                    }
                }
                None => Response::err(id, -32601, format!("Unknown tool: {name}")),
            }
        }
        _ => Response::err(id, -32601, format!("Unknown method: {method}")),
    }
}
