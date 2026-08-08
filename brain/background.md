---
slug: background
title: Project background
role: project background
updated: "2026-07-23T13:55:05"
---

# Project background

---
slug: background
title: Project background
role: project background
updated: "2026-07-23T13:52:55"
---

# Project background

**mcp-for-page-builders is the Rust MCP server providing the WordPress/Elementor page-building tools used in this very session** (`mcp_mcp_for_page_builders_*`). It gives AI assistants structured, element-level access to WordPress + Elementor instead of having them fumble with raw HTML/PHP or the Elementor visual editor.

Direct quote (README): "5.4MB binary · ~3MB RAM · ~1ms startup · 75 tools" — verified: `src/tools/mod.rs`'s tool registry contains exactly 75 `Box::new(...)` entries.

## What it actually does (verified via source, not just README)

Element-level CRUD on Elementor pages (get/add/update/remove/duplicate/move elements, patch multiple at once), widget schema validation with "did you mean?" suggestions, global design tokens (colors/typography/kit defaults/schema), Theme Builder templates (header/footer/single/archive/popup/loop-item), CDP-based visual tools (screenshot, visual_compare, visual_diff, ui_quality_audit, inspect_page, clone_element from a live external page, match_styles), CSS→Elementor settings conversion, custom widget scaffolding via a companion WordPress plugin, local dev-env detection (DDEV/Lando auto-boot), multi-site connection management, and full WP REST API passthrough (users/comments/categories/tags/media/search + a generic `wp_api` escape hatch).

## Non-goals / design stance

- `unsafe_code = "forbid"` at the lint level, plus a strict clippy ruleset (all+pedantic+nursery, OOP-favoring restriction lints like `use_self`) — this is a deliberately disciplined Rust codebase, not a quick script.
- Multi-site credentials are persisted to `~/.config/mcp-for-page-builders/sites.json`, not just passed via env vars each session — see [[multi-site-credential-store-design]] for why and how this coexists with the legacy env-var path.
- The `install_config` self-installation feature (see [[install-config-real-writes-vs-printed-snippets]]) is honest about its limits: it only *actually writes files* for Kiro and project-scope Claude Code/Codex; everywhere else it prints a snippet for the human to apply.

See architecture for the module layout and [[mcp-bridge-plugin-purpose]] for the companion WordPress plugin this server can install.
