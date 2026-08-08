# Copilot instructions for mcp-for-page-builders

## Build, lint, and test commands

- Build debug binary: `cargo build`
- Build release binary: `cargo build --release`
- Lint with CI settings: `cargo clippy -- -D warnings`

### Tests

- Full local cycle, including Docker WordPress startup and teardown: `./tests/run.sh`
- Unit tests only, no Docker required: `./tests/run.sh --unit`
- Integration tests against the managed Docker environment: `./tests/run.sh --retest`
- E2E tests against the managed Docker environment: `./tests/run.sh --e2e`
- Full suite while keeping WordPress running afterward: `./tests/run.sh --keep`
- Stop and remove the managed test environment: `./tests/run.sh --down`

Run single tests with Cargo filters:

- Unit test: `cargo test --test unit find_root_element`
- Resilience test: `cargo test --test resilience survives_blank_lines_and_malformed_json`
- Integration test, after exporting `WP_TEST_URL`, `WP_TEST_USER`, and `WP_TEST_PASS`: `cargo test --test integration tool_get_page -- --test-threads=1`
- E2E test, after exporting the same WordPress env vars: `cargo test --test e2e widget_heading -- --test-threads=1`

CI runs `cargo test --test unit`, integration and E2E tests sequentially against `juslintek/wp-sqlite-elementor-server:latest`, `cargo clippy -- -D warnings`, and `cargo build --release`.

## High-level architecture

This repository is a Rust 2024 MCP server for WordPress page builders, currently focused on Elementor. The binary speaks line-delimited JSON-RPC over stdio and exposes MCP tools for page/post CRUD, Elementor element tree mutation, global design tokens, schema validation, visual/CDP inspection, bridge plugin installation, multi-site management, and setup/auth flows.

The MCP request path is centered in `src/main.rs`, `src/mcp.rs`, `src/types/`, and `src/tools/mod.rs`. `main.rs` handles `initialize`, `tools/list`, and `tools/call`, recovers from per-request panics, and drains server notifications. `mcp.rs` owns stdio JSON-RPC transport and intentionally skips blank/malformed lines without closing the connection. Every tool implements the `Tool` trait from `src/types/tool.rs`; adding a tool requires both an implementation and registration in `tools::all_tools()`.

WordPress access flows through `src/wp.rs`. `WpClient` wraps reqwest, Basic Auth, unconfigured/CDP-only mode, a shared multi-site credential store, and reconfiguration notifications. Site credentials are persisted under `~/.config/mcp-for-page-builders/`, while `WP_URL`, `WP_APP_USER`, `WP_APP_PASSWORD`, `WP_TLS_INSECURE`, and `CHROME_PATH` remain supported runtime overrides.

Elementor data is handled as a nested `Element` tree in `src/types/element.rs` and `src/elementor/`. WordPress stores `_elementor_data` as a JSON string in post meta, so create/update/template APIs validate and serialize element arrays as strings. Element reads/writes try standard REST endpoints first (`pages`, `posts`, `elementor_library`, `udesign_template`) and then bridge postmeta endpoints when needed.

Visual tooling is built on Chrome DevTools Protocol in `src/cdp.rs` and `src/tools/visual/`. Chrome launches lazily, uses PID-scoped profiles for parallel MCP server instances, and recovers by resetting the shared browser on navigation failures. Screenshots are saved as full-quality PNG files but returned inline as JPEG to keep JSON-RPC response lines within MCP client buffer limits.

Runtime resilience is deliberate. `src/logging.rs` reserves stdout for JSON-RPC only and writes durable logs to `~/.config/mcp-for-page-builders/logs/`. `src/session.rs` maintains a lock file and operation journal so pending write operations can be surfaced through session-state tools after crashes or restarts.

## Key conventions

- Keep stdout free of diagnostics in MCP server mode; use stderr or tracing because stdout is the JSON-RPC transport.
- Tool definitions live next to tool implementations. Keep `ToolDef` names, descriptions, and JSON input schemas synchronized with `run()` behavior.
- Return user-visible tool failures as `ToolResult::error(...)` when the MCP call itself succeeded; reserve JSON-RPC errors for protocol-level failures such as unknown methods or tools.
- Parse common scalar tool arguments through `src/args.rs` helpers (`str_arg`, `u64_arg`, `usize_arg`) instead of repeating ad hoc extraction.
- Mutating Elementor/page/template operations should clear Elementor CSS cache, usually through `ElementorService::clear_cache()` or `set_page_elements()`.
- Recoverable write operations should use `wp.session.record(...)` and complete the journal entry after the remote write succeeds.
- Generated Elementor element IDs are 7-character lowercase hex strings from `elementor::generate_id()`; duplication/cloning paths regenerate IDs recursively.
- `update_element` merges settings shallowly into the existing element settings object rather than replacing the whole settings object.
- Multi-site tools that change the active connection should save `SiteStore` and notify `ServerNotification::Reconfigure` so the running `WpClient` hot-reloads.
- Visual tools should support `pre_js`/`wait_ms` when adding page-state-dependent capture behavior, and local URL failures should point users toward `ensure_site`.
- Integration and E2E tests depend on `WP_TEST_URL`, `WP_TEST_USER`, and `WP_TEST_PASS`; `tests/run.sh` manages these via `.test-env` when using the Docker test environment.

## MCP servers useful in this repo

Project MCP configuration lives in `.vscode/mcp.json`.

- Use `playwright` for browser automation and repeatable interaction checks against local WordPress/Elementor pages.
- Use the local `mcp-for-page-builders` server entry to exercise this repository's MCP tools from a client; it runs `cargo run --release --`, so set WordPress env vars or use the server's setup/site tools when WordPress-specific tools are needed.
- When `visual-parity-mcp` is available in the session, prefer it for quick screenshots, responsive captures, visual comparisons, Lighthouse/accessibility audits, DOM snapshots, computed styles, and color contrast checks across arbitrary URLs.
