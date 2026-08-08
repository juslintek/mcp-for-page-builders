---
id: mcp-bridge-plugin-purpose
title: "The MCP Bridge WordPress plugin: why get/set_wp_option needs it"
category: concept
status: active
created: "2026-07-23T13:54:14"
updated: "2026-07-23T13:54:31"
---

## compiled_truth

Per README (not independently verified against `install_bridge.rs`'s actual implementation during this pass): WordPress core's REST API only exposes a small allowlisted set of options via `wp/v2/settings`. The companion "MCP Bridge" WordPress plugin adds two REST endpoints WP core doesn't provide: a status/health-check endpoint, and an arbitrary-option read/write endpoint (`mcp-for-page-builders/v1/option`). Without the bridge installed, `get_wp_option`/`set_wp_option` fall back to `wp/v2/settings` and can only touch that small allowlisted subset — with the bridge, they can read/write anything (Elementor internals, Theme Builder conditions, third-party plugin settings).

**Install chain (README's description, 5 steps, not source-verified):** check if already active → try auto-install from wordpress.org → activate if already uploaded but inactive → bootstrap via the Code Snippets plugin as a no-credential deployment channel, then remove Code Snippets afterward → if all else fails, return the raw PHP for manual install.

**If this page is ever wrong**, the authoritative source is `src/tools/bridge/install_bridge.rs` (not read during this brain-bootstrap pass) — read that file directly rather than trusting this summary if the bridge behaves differently than described.


## timeline

- time: 2026-07-23T13:54:14
  kind: decision
  summary: "Created this page: The MCP Bridge WordPress plugin: why get/set_wp_option needs it"
  source: "README.md (not independently verified against install_bridge.rs source)"
  affects: [mcp-bridge-plugin-purpose]

- time: 2026-07-23T13:54:31
  kind: decision
  summary: "captured during brain-bootstrap from README.md; NOT independently verified against install_bridge.rs source"
  source: README.md
  affects: [mcp-bridge-plugin-purpose]
