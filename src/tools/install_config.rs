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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    Kiro, ClaudeCode, ClaudeDesktop, GeminiCli, CodexCli, Cursor, Windsurf, Cline, Unknown,
}

impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Kiro => write!(f, "Kiro CLI"),
            Self::ClaudeCode => write!(f, "Claude Code CLI"),
            Self::ClaudeDesktop => write!(f, "Claude Desktop"),
            Self::GeminiCli => write!(f, "Gemini CLI"),
            Self::CodexCli => write!(f, "Codex CLI"),
            Self::Cursor => write!(f, "Cursor"),
            Self::Windsurf => write!(f, "Windsurf"),
            Self::Cline => write!(f, "Cline"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

fn home() -> PathBuf {
    std::env::var("HOME").map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

fn binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/path/to/mcp-for-page-builders".into())
}

fn detect_host() -> Host {
    // Kiro: KIRO_AGENT_PATH or kiro-gateway API key
    if std::env::var("KIRO_AGENT_PATH").is_ok() { return Host::Kiro; }
    if std::env::var("ANTHROPIC_API_KEY").ok().is_some_and(|k| k.starts_with("kiro-gateway")) {
        return Host::Kiro;
    }
    // Claude Code: CLAUDE_CODE env or claude binary in parent
    if std::env::var("CLAUDE_CODE").is_ok() || std::env::var("CLAUDE_SESSION_ID").is_ok() {
        return Host::ClaudeCode;
    }
    // Codex: CODEX_HOME or parent process
    if std::env::var("CODEX_HOME").is_ok() { return Host::CodexCli; }
    // Gemini CLI: GEMINI_API_KEY with CLI context
    if std::env::var("GEMINI_CLI").is_ok() { return Host::GeminiCli; }
    // Cursor: CURSOR_SESSION or known path
    if std::env::var("CURSOR_SESSION").is_ok() { return Host::Cursor; }
    // Windsurf
    if std::env::var("WINDSURF_SESSION").is_ok() { return Host::Windsurf; }
    // Fallback: check config file existence
    if home().join(".claude.json").exists() || home().join(".claude/settings.json").exists() {
        return Host::ClaudeCode;
    }
    if home().join(".codex/config.toml").exists() { return Host::CodexCli; }
    if home().join(".gemini/settings.json").exists() { return Host::GeminiCli; }
    Host::Unknown
}

fn write_file(path: &Path, content: &str) -> Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(format!("  ✓ {}", path.display()))
}

fn mcp_json_snippet() -> Value {
    json!({
        "mcp-for-page-builders": {
            "command": binary_path()
        }
    })
}

// ── Kiro ──────────────────────────────────────────────────────────────────────

fn install_kiro(project: bool) -> Result<Vec<String>> {
    let base = if project { PathBuf::from(".kiro") } else { home().join(".kiro") };
    let mut log = Vec::new();
    log.push(write_file(&base.join("skills/mcp-for-page-builders/SKILL.md"), SKILL_MD)?);
    log.push(write_file(&base.join("agents/wordpress.json"), AGENT_JSON)?);
    log.push(write_file(&base.join("agents/prompts/wordpress.md"), AGENT_PROMPT)?);
    // Patch AGENTS.md
    let steering = base.join("steering/AGENTS.md");
    if steering.exists() {
        let content = std::fs::read_to_string(&steering)?;
        if !content.contains("wordpress") {
            let patched = if let Some(pos) = content.find("| `default`") {
                let mut c = content.clone();
                c.insert_str(pos, &format!("{AGENTS_ROW}\n"));
                c
            } else {
                format!("{content}\n{AGENTS_ROW}\n")
            };
            std::fs::write(&steering, patched)?;
            log.push(format!("  ✓ {} (patched)", steering.display()));
        } else {
            log.push(format!("  ○ {} (already has wordpress)", steering.display()));
        }
    }
    Ok(log)
}

fn kiro_instructions(project: bool) -> String {
    let scope = if project { "project (./.kiro/)" } else { "user (~/.kiro/)" };
    match install_kiro(project) {
        Ok(log) => format!(
            "Installed for Kiro CLI at {scope} scope:\n\n{}\n\n\
             Installed:\n  • Skill: mcp-for-page-builders (72 tools)\n  • Agent: wordpress (ctrl+shift+w)\n  • Agent prompt: WordPress & Elementor specialist\n\n\
             Restart your session to activate.",
            log.join("\n")
        ),
        Err(e) => format!("Failed to install Kiro config: {e}"),
    }
}

// ── Claude Code CLI ───────────────────────────────────────────────────────────

fn claude_code_instructions(project: bool) -> String {
    let bin = binary_path();
    if project {
        // Write .mcp.json in project root
        let mcp = json!({ "mcpServers": mcp_json_snippet() });
        match write_file(Path::new(".mcp.json"), &serde_json::to_string_pretty(&mcp).unwrap_or_default()) {
            Ok(line) => format!("Installed for Claude Code CLI (project scope):\n\n{line}\n\nRestart Claude Code to activate."),
            Err(e) => format!("Failed: {e}"),
        }
    } else {
        format!(
            "For Claude Code CLI, run this command in your terminal:\n\n\
             ```bash\nclaude mcp add --transport stdio --scope user mcp-for-page-builders -- {bin}\n```\n\n\
             Or add to project scope:\n\
             ```bash\nclaude mcp add --transport stdio --scope project mcp-for-page-builders -- {bin}\n```\n\n\
             Verify with: `claude mcp list`"
        )
    }
}

// ── Gemini CLI ────────────────────────────────────────────────────────────────

fn gemini_cli_instructions() -> String {
    let bin = binary_path();
    let snippet = serde_json::to_string_pretty(&json!({
        "mcpServers": {
            "mcp-for-page-builders": {
                "command": bin,
                "timeout": 30000,
                "trust": true
            }
        }
    })).unwrap_or_default();
    format!(
        "For Gemini CLI, add to ~/.gemini/settings.json:\n\n```json\n{snippet}\n```\n\n\
         Then restart Gemini CLI. Check with `/mcp` command."
    )
}

// ── Codex CLI ─────────────────────────────────────────────────────────────────

fn codex_cli_instructions(project: bool) -> String {
    let bin = binary_path();
    let toml_snippet = format!(
        "[mcp_servers.mcp-for-page-builders]\ncommand = \"{bin}\"\ntool_timeout_sec = 60"
    );
    if project {
        match write_file(Path::new(".codex/config.toml"), &toml_snippet) {
            Ok(line) => format!("Installed for Codex CLI (project scope):\n\n{line}\n\nRestart Codex to activate."),
            Err(e) => format!("Failed: {e}"),
        }
    } else {
        format!(
            "For Codex CLI, run:\n\n\
             ```bash\ncodex mcp add mcp-for-page-builders -- {bin}\n```\n\n\
             Or add to ~/.codex/config.toml:\n\n```toml\n{toml_snippet}\n```\n\n\
             Verify with: `codex mcp` or `/mcp` in TUI."
        )
    }
}

// ── JSON-based hosts (Cursor, Windsurf, Cline, Claude Desktop) ────────────────

fn json_host_instructions(host: Host) -> String {
    let snippet = serde_json::to_string_pretty(&json!({ "mcpServers": mcp_json_snippet() })).unwrap_or_default();
    let (file, note) = match host {
        Host::ClaudeDesktop => {
            let p = if cfg!(target_os = "macos") {
                "~/Library/Application Support/Claude/claude_desktop_config.json"
            } else {
                "%APPDATA%\\Claude\\claude_desktop_config.json"
            };
            (p, "Restart Claude Desktop to activate.")
        }
        Host::Cursor => (".cursor/mcp.json (project root)", "Restart Cursor to activate."),
        Host::Windsurf => ("~/.codeium/windsurf/mcp_config.json", "Restart Windsurf to activate."),
        Host::Cline => (".vscode/mcp.json or Cline MCP settings", "Restart Cline to activate."),
        _ => ("your MCP config file", "Restart your client to activate."),
    };
    format!(
        "For {host}, add to {file}:\n\n```json\n{snippet}\n```\n\n\
         No credentials needed in config — the server manages its own storage.\n\
         Use `connect_site` tool after restart to add WordPress sites.\n\n{note}"
    )
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
                        "description": "Where to install: 'user' (~/.kiro/) or 'project' (./.kiro/ in CWD). Default: user.",
                        "default": "user"
                    }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let host = detect_host();
        let project = args.get("scope").and_then(Value::as_str) == Some("project");

        let mut sections = Vec::new();
        sections.push(format!("Detected host: **{host}**\n"));

        match host {
            Host::Kiro => {
                sections.push(kiro_instructions(project));
            }
            Host::ClaudeCode => {
                sections.push(claude_code_instructions(project));
                // Also install Kiro config if Kiro is present (dual-use)
                if home().join(".kiro").exists() {
                    sections.push(String::new());
                    sections.push("Also found Kiro config — installing skill/agent there too:".into());
                    sections.push(kiro_instructions(false));
                }
            }
            Host::GeminiCli => {
                sections.push(gemini_cli_instructions());
            }
            Host::CodexCli => {
                sections.push(codex_cli_instructions(project));
            }
            Host::ClaudeDesktop | Host::Cursor | Host::Windsurf | Host::Cline => {
                sections.push(json_host_instructions(host));
            }
            Host::Unknown => {
                // Install Kiro config (most capable) + show generic instructions
                if home().join(".kiro").exists() {
                    sections.push(kiro_instructions(project));
                    sections.push(String::new());
                }
                sections.push("For other MCP clients, here are the config snippets:\n".into());
                sections.push(format!("**Claude Code CLI:**\n```bash\nclaude mcp add --transport stdio mcp-for-page-builders -- {}\n```\n", binary_path()));
                sections.push(format!("**Codex CLI:**\n```bash\ncodex mcp add mcp-for-page-builders -- {}\n```\n", binary_path()));
                sections.push(format!("**Gemini CLI** (add to ~/.gemini/settings.json):\n**Cursor/Windsurf/Claude Desktop** (add to their mcp.json):\n```json\n{}\n```",
                    serde_json::to_string_pretty(&json!({"mcpServers": mcp_json_snippet()})).unwrap_or_default()
                ));
            }
        }

        Ok(ToolResult::text(sections.join("\n")))
    }
}
