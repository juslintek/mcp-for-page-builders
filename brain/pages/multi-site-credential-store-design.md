---
id: multi-site-credential-store-design
title: "Credentials persist to a JSON file (~/.config), not just env vars; sites can be switched without a restart"
category: decision
status: active
created: "2026-07-23T13:54:14"
updated: "2026-07-23T13:54:31"
---

## compiled_truth

Verified by reading `wp.rs` and `main.rs` directly (not just README):

**`SiteStore`** = `HashMap<String, SiteCredentials>` (keyed by URL) + an `active: Option<String>` pointer, serialized as pretty JSON to `~/.config/mcp-for-page-builders/sites.json`. This is a **deliberate design choice to persist credentials across sessions** rather than requiring env vars every time — but the legacy env-var path (`WP_URL`/`WP_APP_USER`/`WP_APP_PASSWORD`) still works and is merged into the store on startup, made active if present. Both paths coexist; neither has been deprecated.

The store is wrapped in `Arc<RwLock<SiteStore>>` and handed to the tool layer, so `connect_site`/`switch_site`/`disconnect_site`/`list_sites` tools can mutate it at runtime.

**Switching sites does not restart the process.** A site-switching tool mutates the store, then sends a `ServerNotification::Reconfigure` over an mpsc channel back to the `main.rs` event loop, which drains it and calls `wp.reconfigure(new_creds)` — rebuilding the reqwest client, Basic Auth header, and base URL **in place**. The MCP client sees no restart, no dropped connection, no stale-site error on the next call.

**Why this matters for future work:** if adding a new credential field or auth mechanism, it needs to flow through both the env-var merge path AND the `SiteStore`/`reconfigure()` path to stay consistent — a change to only one would create a silent divergence between "fresh env-var startup" and "switched via tool call" behavior.


## timeline

- time: 2026-07-23T13:54:14
  kind: decision
  summary: "Created this page: Credentials persist to a JSON file (~/.config), not just env vars; sites can be switched without a restart"
  source: "src/wp.rs, src/main.rs, read directly"
  affects: [multi-site-credential-store-design]

- time: 2026-07-23T13:54:31
  kind: decision
  summary: captured during brain-bootstrap via direct reading of src/wp.rs and src/main.rs
  source: "src/wp.rs, src/main.rs"
  affects: [multi-site-credential-store-design]
