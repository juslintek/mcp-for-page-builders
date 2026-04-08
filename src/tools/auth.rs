use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use crate::mcp::{ToolDef, ToolResult};
use crate::util::{config_path, urldecode, urlencode, uuid};
use crate::wp::WpClient;
use super::Tool;

const APP_NAME: &str = "elementor-mcp";
const TIMEOUT_SECS: u64 = 300;

pub struct Authenticate;

#[async_trait]
impl Tool for Authenticate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "authenticate",
            description: "Start authentication flow with a WordPress site. Opens a local web page where you enter the WordPress URL, then redirects to WordPress for Application Password approval. Returns the URL to open in a browser. Credentials are saved automatically.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "wp_url": {
                        "type": "string",
                        "description": "WordPress site URL. If omitted, a local form will ask for it."
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let preset_url = args.get("wp_url").and_then(|v| v.as_str()).map(std::string::ToString::to_string);

        // Bind to a random port
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = std_listener.local_addr()?.port();
        std_listener.set_nonblocking(true)?;

        let done = Arc::new(Notify::new());
        let result: Arc<tokio::sync::Mutex<Option<AuthResult>>> = Arc::new(tokio::sync::Mutex::new(None));

        // Spawn HTTP server on a dedicated thread
        let preset_clone = preset_url.clone();
        let result2 = result.clone();
        let done2 = done.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let listener = TcpListener::from_std(std_listener).unwrap();
                serve(listener, port, preset_clone, result2, done2).await;
            });
        });

        let base = format!("http://localhost:{port}");

        let open_url = format!("{base}/");
        let msg = format!(
            "Open this URL in your browser to authenticate:\n\n  {open_url}\n\nWaiting for approval (timeout: {TIMEOUT_SECS}s)..."
        );

        // Wait for auth to complete or timeout
        let completed = tokio::time::timeout(
            std::time::Duration::from_secs(TIMEOUT_SECS),
            done.notified(),
        ).await;

        if completed.is_err() {
            return Ok(ToolResult::error(format!("Authentication timed out after {TIMEOUT_SECS}s.\nOpen {open_url} to try again.")));
        }

        let auth = result.lock().await;
        match auth.as_ref() {
            Some(AuthResult { wp_url, user, password, error: None }) => {
                // Save config
                save_config(wp_url, user, password)?;
                Ok(ToolResult::text(format!(
                    "Authenticated successfully!\n\nSite: {wp_url}\nUser: {user}\nCredentials saved to: {}\n\nTo use: set WP_URL={wp_url} WP_APP_USER={user} WP_APP_PASSWORD={password}",
                    config_path(wp_url).display()
                )))
            }
            Some(AuthResult { error: Some(e), .. }) => {
                Ok(ToolResult::error(format!("Authentication failed: {e}\nOpen {open_url} to try again.")))
            }
            _ => Ok(ToolResult::text(msg)),
        }
    }
}

struct AuthResult {
    wp_url: String,
    user: String,
    password: String,
    error: Option<String>,
}

async fn serve(
    listener: TcpListener, port: u16,
    preset_url: Option<String>,
    result: Arc<tokio::sync::Mutex<Option<AuthResult>>>,
    done: Arc<Notify>,
) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { break };
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = request.lines().next().unwrap_or("")
            .split_whitespace().nth(1).unwrap_or("/");

        let response = if path == "/" || path == "/index.html" {
            page_form(port, preset_url.as_deref())
        } else if path.starts_with("/connect") {
            // Parse form POST body or query params
            let wp_url = extract_param(&request, "wp_url").unwrap_or_default();
            if wp_url.is_empty() {
                page_error("Please enter a WordPress URL")
            } else {
                page_redirect(&wp_url, port)
            }
        } else if path.starts_with("/callback") {
            let user = extract_query(path, "user_login");
            let password = extract_query(path, "password");

            if let (Some(user), Some(password)) = (user, password) {
                let wp_url = extract_query(path, "site_url")
                    .or_else(|| preset_url.clone())
                    .unwrap_or_default();
                *result.lock().await = Some(AuthResult {
                    wp_url, user, password, error: None,
                });
                done.notify_one();
                page_success()
            } else {
                *result.lock().await = Some(AuthResult {
                    wp_url: String::new(), user: String::new(), password: String::new(),
                    error: Some("Authorization was rejected".into()),
                });
                done.notify_one();
                page_rejected()
            }
        } else {
            http_response("404 Not Found", "text/plain", "Not found")
        };

        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.flush().await;
    }
}

fn extract_param(request: &str, key: &str) -> Option<String> {
    // Try POST body first
    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
    extract_from_qs(body, key).or_else(|| {
        // Try query string
        let path = request.lines().next()?.split_whitespace().nth(1)?;
        extract_query(path, key)
    })
}

fn extract_query(path: &str, key: &str) -> Option<String> {
    let qs = path.split('?').nth(1)?;
    extract_from_qs(qs, key)
}

fn extract_from_qs(qs: &str, key: &str) -> Option<String> {
    for pair in qs.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            return Some(urldecode(kv.next()?));
        }
    }
    None
}

fn save_config(wp_url: &str, user: &str, password: &str) -> Result<()> {
    let path = config_path(wp_url);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let config = json!({"wp_url": wp_url, "wp_user": user, "wp_app_password": password});
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len())
}

fn page_form(_port: u16, preset: Option<&str>) -> String {
    let val = preset.unwrap_or("");
    http_response("200 OK", "text/html", &format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Elementor MCP — Connect</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:system-ui,sans-serif;background:#0f0f23;color:#e0e0e0;display:flex;justify-content:center;align-items:center;min-height:100vh}}
.card{{background:#1a1a2e;border-radius:12px;padding:48px;max-width:480px;width:100%;box-shadow:0 8px 32px rgba(0,0,0,.4)}}
h1{{font-size:24px;margin-bottom:8px;color:#fff}}
p{{color:#888;margin-bottom:24px;font-size:14px}}
label{{display:block;font-size:13px;color:#aaa;margin-bottom:6px}}
input{{width:100%;padding:12px 16px;border:1px solid #333;border-radius:8px;background:#16213e;color:#fff;font-size:16px;outline:none}}
input:focus{{border-color:#e94560}}
button{{width:100%;padding:14px;border:none;border-radius:8px;background:#e94560;color:#fff;font-size:16px;font-weight:600;cursor:pointer;margin-top:16px}}
button:hover{{background:#c73e54}}
.logo{{font-size:32px;margin-bottom:16px}}
</style></head><body>
<div class="card">
<div class="logo">🔌</div>
<h1>Connect WordPress</h1>
<p>Enter your WordPress site URL to authorize Elementor MCP access.</p>
<form method="POST" action="/connect">
<label for="wp_url">WordPress URL</label>
<input type="url" name="wp_url" id="wp_url" value="{val}" placeholder="https://my-site.com" required>
<button type="submit">Connect →</button>
</form>
</div></body></html>"#))
}

fn page_redirect(wp_url: &str, port: u16) -> String {
    let wp_url = wp_url.trim_end_matches('/');
    let callback = urlencode(&format!("http://localhost:{port}/callback?site_url={}", urlencode(wp_url)));
    let auth_url = format!(
        "{wp_url}/wp-admin/authorize-application.php?app_name={APP_NAME}&app_id={}&success_url={callback}&reject_url={callback}",
        uuid()
    );
    http_response("302 Found", "text/html", &format!(
        r#"<html><head><meta http-equiv="refresh" content="0;url={auth_url}"></head><body>Redirecting to WordPress...</body></html>"#
    )).replace("\r\nConnection:", &format!("\r\nLocation: {auth_url}\r\nConnection:"))
}

fn page_success() -> String {
    http_response("200 OK", "text/html", r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Connected!</title>
<style>body{font-family:system-ui;background:#0f0f23;color:#e0e0e0;display:flex;justify-content:center;align-items:center;min-height:100vh}
.card{background:#1a1a2e;border-radius:12px;padding:48px;text-align:center;max-width:400px}
h1{color:#4ade80;margin-bottom:12px}.icon{font-size:64px;margin-bottom:16px}p{color:#888}</style></head>
<body><div class="card"><div class="icon">✓</div><h1>Connected!</h1><p>You can close this tab. The MCP server is now authenticated.</p></div></body></html>"#)
}

fn page_rejected() -> String {
    http_response("200 OK", "text/html", r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Rejected</title>
<style>body{font-family:system-ui;background:#0f0f23;color:#e0e0e0;display:flex;justify-content:center;align-items:center;min-height:100vh}
.card{background:#1a1a2e;border-radius:12px;padding:48px;text-align:center;max-width:400px}
h1{color:#f87171;margin-bottom:12px}.icon{font-size:64px;margin-bottom:16px}p{color:#888}</style></head>
<body><div class="card"><div class="icon">✗</div><h1>Rejected</h1><p>Authorization was denied. You can close this tab.</p></div></body></html>"#)
}

fn page_error(msg: &str) -> String {
    http_response("400 Bad Request", "text/html", &format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Error</title></head><body><h1>Error</h1><p>{msg}</p><a href="/">Try again</a></body></html>"#))
}
