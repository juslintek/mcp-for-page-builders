---
name: mcp-for-page-builders
description: Work with WordPress and Elementor through the local MCP server, especially native authoring, safe conversion, and rendered-page review.
---

# MCP for Page Builders

Use the connected site's live schemas and permissions. Inspect before editing, choose Elementor-native structures when they represent the request, and verify saved state and guest rendering after a write. A successful response or DOM check alone is not acceptance.

## Setup

Build with `cargo build --release`, then add the binary as an MCP server in the client. Keep credentials in the local client configuration; do not print or expose them. Connect with `connect_site` or `authenticate`, then confirm the target using `list_sites` and `switch_site`.

## References

- Read [Elementor workflow](references/elementor-workflow.md) for native authoring, conversion, resource revisions, and visual acceptance.
- Read [Karjera review](references/karjera-review.md) for dated live recovery evidence and separate clone, migration, and package gates.
- Read [MCP development](references/mcp-development.md) when changing tools, schemas, persistence, or checks.

Keep inspection read-only until the intended change is understood. Preserve unsupported content, report partial failures with operation and restore pointers, and distinguish source checks, isolated native fixtures, actual page saves, installed-MCP acceptance, exact-version upgrades, and deployed behavior. Evidence at one stage does not accept the next. Never infer native support or visual parity without evidence from the target site.
