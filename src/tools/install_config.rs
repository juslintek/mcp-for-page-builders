use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::WpClient;

const SKILL_MD: &str = include_str!("../../assets/SKILL.md");
const AGENT_JSON: &str = include_str!("../../assets/agent.json");
const AGENT_PROMPT: &str = include_str!("../../assets/agent_prompt.md");
const AGENTS_ROW: &str = include_str!("../../assets/agents_md_row.txt");

pub struct InstallConfig;

#[derive(Debug, Clone, Copy)]
enum Host { Kiro, ClaudeDesktop, Cursor, Windsurf, Cline, Unknown }

#[derive(Debug, Clone, Copy)]
enum Scope { User, Project }

fn detect_host() -> Host {
    if std::env::var("KIRO_AGENT_PATH").is_ok() { return Host::Kiro; }
    if std::env::var("ANTHROPIC_API_KEY").ok().is_some_and(|k| k.starts_with("kiro-gateway")) {
        return Host::Kiro;
    }
    // Other hosts can be detected by parent process or config presence
    if home_dir().join("Library/Application Support/Claude/claude_desktop_config.json").exists() {
        return Host::ClaudeDesktop;
    }
    Host::Unknown
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

fn kiro_base(scope: Scope) -> PathBuf {
    match scope {
        Scope::User => home_dir().join(".kiro"),
        Scope::Project => PathBuf::from(".kiro"),
    }
}

fn write_file(path: &Path, content: &str) -> Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(format!("  ✓ {}", path.display()))
}

fn install_kiro(scope: Scope) -> Result<Vec<String>> {
    let base = kiro_base(scope);
    let mut log = Vec::new();

    // Skill
    log.push(write_file(&base.join("skills/mcp-for-page-builders/SKILL.md"), SKILL_MD)?);

    // Agent
    log.push(write_file(&base.join("agents/wordpress.json"), AGENT_JSON)?);
    log.push(write_file(&base.join("agents/prompts/wordpress.md"), AGENT_PROMPT)?);

    // Patch AGENTS.md — add row if not already present
    let steering = base.join("steering/AGENTS.md");
    if steering.exists() {
        let content = std::fs::read_to_string(&steering)?;
        if !content.contains("wordpress") {
            // Insert after the last agent row (before the empty line after the table)
            let patched = if let Some(pos) = content.find("| `default`") {
                let mut c = content.clone();
                c.insert_str(pos, &format!("{AGENTS_ROW}\n"));
                c
            } else {
                // Append to end of file
                format!("{content}\n{AGENTS_ROW}\n")
            };
            std::fs::write(&steering, patched)?;
            log.push(format!("  ✓ {} (patched)", steering.display()));
        } else {
            log.push(format!("  ○ {} (already has wordpress agent)", steering.display()));
        }
    } else {
        log.push(format!("  ○ {} (not found, skipped)", steering.display()));
    }

    Ok(log)
}

fn mcp_json_snippet() -> String {
    let binary = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/path/to/mcp-for-page-builders".into());

    serde_json::to_string_pretty(&json!({
        "mcpServers": {
            "mcp-for-page-builders": {
                "command": binary
            }
        }
    })).unwrap_or_default()
}

#[async_trait]
impl Tool for InstallConfig {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "install_config",
            description: "Install skill, agent, and prompt files for this MCP server. Auto-detects the host agent (Kiro, Claude Desktop, Cursor, etc.) and installs to the chosen scope (user or project).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["user", "project"],
                        "description": "Where to install: 'user' (~/.kiro/) or 'project' (./.kiro/ in CWD). Default: user."
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let host = detect_host();
        let scope = match args.get("scope").and_then(Value::as_str) {
            Some("project") => Scope::Project,
            _ => Scope::User,
        };

        match host {
            Host::Kiro => {
                let scope_label = match scope {
                    Scope::User => "user (~/.kiro/)",
                    Scope::Project => "project (./.kiro/)",
                };
                let log = install_kiro(scope)?;
                Ok(ToolResult::text(format!(
                    "Installed mcp-for-page-builders config for Kiro at {scope_label} scope:\n\n{}\n\n\
                     Installed:\n\
                     • Skill: mcp-for-page-builders (71 tools)\n\
                     • Agent: wordpress (ctrl+shift+w)\n\
                     • Agent prompt: WordPress & Elementor specialist\n\n\
                     Restart your session to activate the new agent.",
                    log.join("\n")
                )))
            }
            Host::ClaudeDesktop => {
                Ok(ToolResult::text(format!(
                    "Detected: Claude Desktop\n\n\
                     Claude Desktop doesn't support skills/agents — only MCP server config.\n\
                     Add this to ~/Library/Application Support/Claude/claude_desktop_config.json:\n\n\
                     ```json\n{}\n```\n\n\
                     The server manages credentials internally — no env vars needed.\n\
                     Use `connect_site` tool after restart to add WordPress sites.",
                    mcp_json_snippet()
                )))
            }
            Host::Cursor | Host::Windsurf | Host::Cline => {
                let host_name = format!("{host:?}");
                Ok(ToolResult::text(format!(
                    "Detected: {host_name}\n\n\
                     Add this to your MCP config (.cursor/mcp.json or equivalent):\n\n\
                     ```json\n{}\n```\n\n\
                     The server manages credentials internally — no env vars needed.\n\
                     Use `connect_site` tool after restart to add WordPress sites.",
                    mcp_json_snippet()
                )))
            }
            Host::Unknown => {
                // Offer both options
                let log = install_kiro(scope)?;
                Ok(ToolResult::text(format!(
                    "Could not detect host agent. Installed Kiro config (most capable):\n\n{}\n\n\
                     If you're using a different client, add this to your MCP config:\n\n\
                     ```json\n{}\n```",
                    log.join("\n"),
                    mcp_json_snippet()
                )))
            }
        }
    }
}
