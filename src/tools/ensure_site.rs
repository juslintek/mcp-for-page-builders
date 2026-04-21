use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct EnsureSite;

#[derive(Debug)]
pub enum EnvType { Ddev, Lando, Local, Remote }

impl std::fmt::Display for EnvType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Ddev => write!(f, "ddev"), Self::Lando => write!(f, "lando"), Self::Local => write!(f, "local"), Self::Remote => write!(f, "remote") }
    }
}

/// Detect environment type from URL hostname.
pub fn detect_env(url: &str) -> (EnvType, Option<String>) {
    let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("").split(':').next().unwrap_or("");
    if host.ends_with(".ddev.site") {
        let project = host.trim_end_matches(".ddev.site").to_string();
        (EnvType::Ddev, Some(project))
    } else if host.ends_with(".lndo.site") {
        let project = host.trim_end_matches(".lndo.site").to_string();
        (EnvType::Lando, Some(project))
    } else if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" {
        (EnvType::Local, None)
    } else {
        (EnvType::Remote, None)
    }
}

/// Check if a URL is reachable. Returns Ok(status_code) or Err.
pub async fn check_reachable(url: &str) -> Result<u16> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client.head(url).send().await?;
    Ok(resp.status().as_u16())
}

async fn ddev_ensure(project: &str) -> Result<Vec<String>> {
    let mut issues = Vec::new();

    // Check if ddev is available
    let which = tokio::process::Command::new("which").arg("ddev").output().await;
    if which.is_err() || !which.unwrap().status.success() {
        anyhow::bail!("ddev not found in PATH");
    }

    // Check project status
    let list = tokio::process::Command::new("ddev").arg("list").output().await?;
    let list_out = String::from_utf8_lossy(&list.stdout);

    // Find the project line — ddev list output has project names in the table
    let project_line = list_out.lines().find(|l| {
        let lower = l.to_lowercase();
        lower.contains(project) || lower.contains(&project.replace('-', "_"))
    });

    match project_line {
        None => {
            issues.push(format!("DDEV project '{project}' not found. Available projects shown in ddev list."));
        }
        Some(line) => {
            if line.contains("stopped") || line.contains("paused") {
                issues.push(format!("DDEV project '{project}' is stopped — starting..."));
                // Find the project directory from ddev list or try starting by name
                let start = tokio::process::Command::new("ddev")
                    .args(["start", project])
                    .output().await?;
                if !start.status.success() {
                    let err = String::from_utf8_lossy(&start.stderr);
                    issues.push(format!("ddev start failed: {}", err.trim()));
                } else {
                    issues.push("DDEV project started successfully.".into());
                }
            }
            // Project is running (OK)
        }
    }

    Ok(issues)
}

async fn lando_ensure(project: &str) -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let which = tokio::process::Command::new("which").arg("lando").output().await;
    if which.is_err() || !which.unwrap().status.success() {
        anyhow::bail!("lando not found in PATH");
    }
    let list = tokio::process::Command::new("lando").arg("list").output().await?;
    let out = String::from_utf8_lossy(&list.stdout);
    if !out.to_lowercase().contains(project) {
        issues.push(format!("Lando project '{project}' not found."));
    }
    Ok(issues)
}

#[async_trait]
impl Tool for EnsureSite {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "ensure_site",
            description: "Check if a URL is reachable and boot local environments if needed. Auto-detects DDEV, Lando, and local dev servers.\n\nWorkflow: Call before screenshot, visual_compare, or visual_diff when working with local dev sites. Automatically starts stopped DDEV/Lando projects.",
            input_schema: json!({"type":"object","required":["url"],"properties":{
                "url":{"type":"string","description":"URL to check and ensure is running"}
            }}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let (env_type, project) = detect_env(&url);
        let mut issues: Vec<String> = Vec::new();

        // Environment-specific boot
        match (&env_type, &project) {
            (EnvType::Ddev, Some(p)) => {
                match ddev_ensure(p).await {
                    Ok(i) => issues.extend(i),
                    Err(e) => issues.push(format!("DDEV check failed: {e}")),
                }
            }
            (EnvType::Lando, Some(p)) => {
                match lando_ensure(p).await {
                    Ok(i) => issues.extend(i),
                    Err(e) => issues.push(format!("Lando check failed: {e}")),
                }
            }
            _ => {}
        }

        // HTTP reachability check
        let status = match check_reachable(&url).await {
            Ok(code) => {
                if code >= 400 && code != 401 {
                    issues.push(format!("HTTP {code} — site returned an error"));
                }
                format!("{code}")
            }
            Err(e) => {
                issues.push(format!("Unreachable: {e}"));
                "unreachable".into()
            }
        };

        let result = json!({
            "url": url,
            "environment": env_type.to_string(),
            "project": project,
            "http_status": status,
            "ready": issues.is_empty(),
            "issues": issues,
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}
