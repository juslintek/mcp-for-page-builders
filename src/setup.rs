use anyhow::{Context, Result};
use std::io::Write;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::util::{config_dir, config_path, urldecode, urlencode, uuid};

const APP_NAME: &str = "mcp-for-page-builders";

/// Run the auto-auth setup flow.
pub async fn run(wp_url: &str) -> Result<()> {
    let wp_url = wp_url.trim_end_matches('/');
    println!("Setting up authentication for {wp_url}");

    // 1. Discover REST API
    println!("Discovering REST API...");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;

    let resp = client.get(format!("{wp_url}/wp-json/"))
        .send().await
        .context("Cannot reach WordPress REST API. Is the URL correct?")?;

    if !resp.status().is_success() {
        anyhow::bail!("REST API returned {}. Is WordPress installed at {wp_url}?", resp.status());
    }

    let api: serde_json::Value = resp.json().await?;
    let name = api["name"].as_str().unwrap_or("WordPress");
    println!("Found: {name}");

    // 2. Check if Application Passwords are supported
    let auth_url = format!("{wp_url}/wp-admin/authorize-application.php");

    // 3. Start callback server on random port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let callback_url = format!("http://localhost:{port}/callback");

    let authorize_url = format!(
        "{auth_url}?app_name={APP_NAME}&app_id={app_id}&success_url={callback}&reject_url={callback}",
        app_id = uuid(),
        callback = urlencode(&callback_url),
    );

    // 4. Open browser
    println!("\nOpening browser for authorization...");
    println!("If it doesn't open, visit:\n  {authorize_url}\n");
    open_browser(&authorize_url);

    println!("Waiting for approval (press Ctrl+C to cancel)...");

    // 5. Wait for callback
    let (mut stream, _) = listener.accept().await?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the GET request for params
    let path = request.lines().next().unwrap_or("")
        .split_whitespace().nth(1).unwrap_or("");

    // Send response to browser
    let html = if path.contains("user_login") {
        "<html><body><h1>&#10004; Authorized!</h1><p>You can close this tab.</p></body></html>"
    } else {
        "<html><body><h1>&#10008; Rejected</h1><p>Authorization was denied.</p></body></html>"
    };
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{html}");
    tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await?;

    // 6. Extract credentials from callback URL
    let params = parse_query(path);
    let user = params.get("user_login")
        .ok_or_else(|| anyhow::anyhow!("Authorization rejected or no user_login in callback"))?;
    let password = params.get("password")
        .ok_or_else(|| anyhow::anyhow!("No password in callback"))?;

    println!("\nAuthorized as: {user}");

    // 7. Verify credentials work
    print!("Verifying... ");
    let auth = format!("Basic {}", base64_encode(&format!("{user}:{password}")));
    let verify = client.get(format!("{wp_url}/wp-json/wp/v2/users/me"))
        .header("Authorization", &auth)
        .send().await?;

    if !verify.status().is_success() {
        anyhow::bail!("Credentials verification failed: {}", verify.status());
    }
    println!("OK");

    // 8. Save config
    let config = serde_json::json!({
        "wp_url": wp_url,
        "wp_user": user,
        "wp_app_password": password,
    });

    std::fs::create_dir_all(config_dir())?;
    let path = config_path(wp_url);
    let mut f = std::fs::File::create(&path)?;
    f.write_all(serde_json::to_string_pretty(&config)?.as_bytes())?;

    println!("\nCredentials saved to {}", path.display());
    println!("\nUsage:");
    println!("  WP_URL={wp_url} WP_APP_USER={user} WP_APP_PASSWORD={password} mcp-for-page-builders");
    println!("\nOr the MCP server will auto-load from {}", path.display());

    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/c", "start", url]).spawn(); }
}

fn parse_query(path: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                map.insert(urldecode(k), urldecode(v));
            }
        }
    }
    map
}

fn base64_encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}
