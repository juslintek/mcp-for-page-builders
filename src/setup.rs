use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

const APP_NAME: &str = "elementor-mcp";

/// Config file location: ~/.config/elementor-mcp/{host}.json
fn config_dir() -> PathBuf {
    dirs().join("elementor-mcp")
}

fn dirs() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
}

fn config_path(wp_url: &str) -> PathBuf {
    let host = wp_url.trim_end_matches('/')
        .replace("https://", "").replace("http://", "")
        .replace(['/', ':', '.'], "_");
    config_dir().join(format!("{host}.json"))
}

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
        app_id = uuid_simple(),
        callback = urlencoding(&callback_url),
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
    println!("  WP_URL={wp_url} WP_APP_USER={user} WP_APP_PASSWORD={password} elementor-mcp");
    println!("\nOr the MCP server will auto-load from {}", path.display());

    Ok(())
}

/// Load saved credentials for a WordPress URL.
pub fn load_config(wp_url: &str) -> Option<(String, String, String)> {
    let path = config_path(wp_url);
    let content = std::fs::read_to_string(&path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some((
        config["wp_url"].as_str()?.to_string(),
        config["wp_user"].as_str()?.to_string(),
        config["wp_app_password"].as_str()?.to_string(),
    ))
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

fn urlencoding(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
            String::from(b as char)
        }
        _ => format!("%{b:02X}"),
    }).collect()
}

fn urldecode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h = chars.next().unwrap_or(0);
            let l = chars.next().unwrap_or(0);
            let hex = String::from_utf8(vec![h, l]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn base64_encode(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn uuid_simple() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        rng.random::<u32>(), rng.random::<u16>(), rng.random::<u16>(),
        rng.random::<u16>(), rng.random::<u64>() & 0xFFFFFFFFFFFF)
}
