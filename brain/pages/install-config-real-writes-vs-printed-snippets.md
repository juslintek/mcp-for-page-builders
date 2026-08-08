---
id: install-config-real-writes-vs-printed-snippets
title: install_config only actually writes files for Kiro and project-scope Claude Code/Codex - everywhere else it prints a snippet
category: reference
status: active
created: "2026-07-23T13:54:15"
updated: "2026-07-23T13:54:57"
---

## compiled_truth

`install_config` detects the calling host (`detect_host()`) via env vars and config-file presence, then behaves differently per host — **only some hosts get real file writes; most just get a printed snippet for the human to apply manually.**

**Actually writes files:**
- **Kiro**: writes `SKILL.md`, `agents/wordpress.json`, `agents/prompts/wordpress.md` under `.kiro/` (project or user scope), content baked into the binary at compile time via `include_str!` from `assets/`. Also patches an existing `~/.kiro/steering/AGENTS.md` by inserting a row if not already present.
- **Claude Code**: project scope writes `.mcp.json` directly. User scope only *prints* a `claude mcp add ...` command — does not invoke it. If `~/.kiro` also exists, additionally installs the Kiro assets (dual-use).
- **Codex CLI**: project scope writes `.codex/config.toml`. User scope prints the equivalent command/TOML snippet.

**Never writes files, only prints a snippet:**
- Gemini CLI (prints JSON for `~/.gemini/settings.json`)
- Claude Desktop (prints JSON + the platform-specific `claude_desktop_config.json` path)
- Cursor (prints `.cursor/mcp.json` snippet)
- Windsurf (prints `~/.codeium/windsurf/mcp_config.json` snippet)
- Cline
- Unknown host (falls back to Kiro-asset install if `~/.kiro` exists, then dumps generic snippets for everything else)

**Why this matters:** the README/tool description framing ("supports all major LLM CLI agents") is accurate about *detection and guidance* coverage, but not about *automatic installation* — don't assume running `install_config` on, say, a Cursor-detected host will actually configure anything; it will only tell the user what to paste where. If a future task needs Cursor/Windsurf/Gemini to get real file writes like Kiro does, that's new work, not a bug fix — the current behavior is intentional (per the detection-vs-write split), not incomplete by oversight per se, but also not documented as a limitation anywhere obvious.


## timeline

- time: 2026-07-23T13:54:15
  kind: decision
  summary: "Created this page: install_config only actually writes files for Kiro and project-scope Claude Code/Codex - everywhere else it prints a snippet"
  source: "src/tools/install_config.rs, read directly"
  affects: [install-config-real-writes-vs-printed-snippets]

- time: 2026-07-23T13:54:57
  kind: decision
  summary: captured during brain-bootstrap via direct reading of src/tools/install_config.rs
  source: "src/tools/install_config.rs, read directly and fully"
  affects: [install-config-real-writes-vs-printed-snippets]
