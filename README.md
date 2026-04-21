# mcp-for-page-builders

A high-performance MCP (Model Context Protocol) server for WordPress page builders — written in Rust.

**5.4MB binary · ~3MB RAM · ~1ms startup · 59 tools**

## Purpose

This project provides AI assistants with deep, structured access to WordPress page builders through the Model Context Protocol. Rather than treating WordPress pages as opaque HTML blobs, it exposes the underlying page builder data model — allowing AI tools to read, create, and modify content at the element level with full awareness of widget types, settings schemas, and design tokens.

### Current State: Elementor

At present, the server is purpose-built for **Elementor** — the most widely used WordPress page builder. It integrates directly with Elementor's native REST API (`elementor/v1/*`) rather than the generic WordPress REST API, giving it access to element trees, widget schemas, global design tokens, experiments, and the live editor via Chrome DevTools Protocol.

Key capabilities in the current Elementor implementation:

- Element-level CRUD — add, update, move, duplicate, or remove individual widgets without replacing the entire page
- Widget schema validation with "did you mean?" suggestions for incorrect setting keys
- Global design token management (colors, typography)
- Elementor kit settings, feature flags, and CSS cache control
- Theme Builder template creation with automatic condition activation
- Visual tools — screenshots with inline images, side-by-side stitched comparison, CDP-based DOM inspection
- CSS-to-Elementor mapping — convert computed styles to Elementor widget settings
- One-shot visual parity — `match_styles` extracts CSS from a reference, converts, and applies in one call
- Live editor control via CDP — open editor, select widgets, change settings, save
- Custom widget scaffolding — generate and deploy PHP widget classes via a companion bridge plugin
- Local environment detection — auto-detect and boot DDEV/Lando projects
- Page interaction before capture — `pre_js` on all visual tools to click menus, dismiss banners, etc.

### Future Direction: Universal Page Builder Support

The long-term vision is to make this server the single MCP integration point for **any WordPress page builder**. When a request comes in, the server will:

1. Detect which page builder(s) are active on the WordPress installation (Elementor, Beaver Builder, Bricks, Divi, Gutenberg blocks, etc.)
2. Automatically route requests to the appropriate handler for that builder
3. Expose a unified tool interface regardless of which builder is installed — so AI assistants don't need to know or care which builder is in use

This means a single MCP configuration will work across different WordPress sites regardless of their page builder choice, and AI tools will be able to work with any builder's data model through a consistent API.

## Quick Start

```bash
# Build from source
git clone https://github.com/juslintek/mcp-for-page-builders
cd mcp-for-page-builders
cargo build --release
```

Add to your MCP client config (e.g. `.kiro/settings/mcp.json`):

```json
{
  "mcpServers": {
    "mcp-for-page-builders": {
      "command": "/path/to/mcp-for-page-builders",
      "env": {
        "WP_URL": "https://your-site.com",
        "WP_APP_USER": "admin",
        "WP_APP_PASSWORD": "xxxx xxxx xxxx xxxx xxxx xxxx"
      }
    }
  }
}
```

### CDP-Only Mode

WordPress credentials are optional. Without `WP_URL`, the server starts in CDP-only mode — all visual and schema tools work, WordPress-dependent tools return a helpful error pointing to `setup_wizard`.

```json
{
  "mcpServers": {
    "mcp-for-page-builders": {
      "command": "/path/to/mcp-for-page-builders",
      "env": { "WP_TLS_INSECURE": "1" }
    }
  }
}
```

### Environment Variables

| Variable | Required | Description |
|---|---|---|
| `WP_URL` | — | WordPress site URL (optional — CDP tools work without it) |
| `WP_APP_USER` | — | WordPress username (defaults to `admin`) |
| `WP_APP_PASSWORD` | — | Application Password (from Users → Profile → Application Passwords) |
| `WP_TLS_INSECURE` | — | Set to any value to accept self-signed certs (DDEV, local dev) |
| `CHROME_PATH` | — | Override Chrome binary path for visual tools |

### Creating an Application Password

```bash
# Via WP-CLI
wp user application-password create admin "mcp-for-page-builders" --porcelain
```

Or: WordPress Admin → Users → Profile → Application Passwords → Add New.

## Tool Reference

### Page Management

| Tool | Description |
|---|---|
| `create_page` | Create a new page with Elementor data |
| `get_page` | Get page by ID including `_elementor_data` |
| `update_page` | Update title, status, and/or Elementor data |
| `delete_page` | Delete a page (optional force bypass trash) |
| `get_page_by_slug` | Look up page ID from URL slug |
| `list_pages` | List pages with IDs, titles, slugs, status |

### Post Management

| Tool | Description |
|---|---|
| `create_post` | Create a new post |
| `get_post` | Get post by ID |
| `list_posts` | List posts |
| `update_post` | Update a post |
| `delete_post` | Delete a post |

### Element Operations

| Tool | Description |
|---|---|
| `get_element_tree` | Flattened view of page structure with paths and IDs |
| `get_element` | Get a single element by ID |
| `add_element` | Insert widget/container at specific position |
| `update_element` | Merge settings into an element (partial update) |
| `remove_element` | Delete element by ID |
| `move_element` | Move element to different parent/position |
| `duplicate_element` | Clone element with new IDs, inserted after original |
| `find_elements` | Search by widget type and/or setting key/value |

### Global Design Tokens

| Tool | Description |
|---|---|
| `get_global_colors` | Get all global colors |
| `set_global_color` | Create or update a global color |
| `delete_global_color` | Delete a global color |
| `get_global_typography` | Get all global typography presets |
| `set_global_typography` | Create or update a typography preset |
| `delete_global_typography` | Delete a typography preset |

### Elementor Settings & Kit

| Tool | Description |
|---|---|
| `get_kit_schema` | All available kit settings with types and defaults |
| `get_kit_defaults` | Default settings applied to each widget type |
| `get_experiments` | All feature flags and their current state |
| `set_experiment` | Enable/disable an experiment (feature flag) |

### Cache

| Tool | Description |
|---|---|
| `clear_cache` | Clear Elementor CSS cache and regenerate styles |

> Cache is cleared automatically after every write operation.

### File I/O

| Tool | Description |
|---|---|
| `download_page` | Save `_elementor_data` to a local JSON file |
| `upload_page` | Update page from a local JSON file |
| `backup_page` | Snapshot current state to timestamped file |

### Visual Tools

Requires Chrome/Chromium installed. All visual tools support `pre_js` for page interaction before capture and return inline images visible to the AI.

| Tool | Description |
|---|---|
| `screenshot` | Full-page screenshot of any URL — returns inline image |
| `screenshot_page` | Screenshot a WordPress page by ID — returns inline image |
| `visual_compare` | Side-by-side stitched comparison of two URLs — returns combined image |
| `visual_diff` | Element-by-element structured comparison with match score — auto-discovers selectors |
| `extract_styles` | Extract computed CSS from a live page element |
| `inspect_page` | Inspect DOM element — bounding box, computed styles, children tree |
| `clone_element` | Clone live DOM element as Elementor JSON |
| `match_styles` | One-shot visual parity: extract CSS → convert to Elementor → apply to element |

#### `pre_js` — Page Interaction Before Capture

All visual tools accept `pre_js` (JavaScript to execute after page load) and `wait_ms` (delay after execution). Use this to click menus, dismiss cookie banners, or expand sections before capture.

```json
{
  "name": "visual_compare",
  "arguments": {
    "url_a": "https://production.com",
    "url_b": "https://dev.ddev.site",
    "pre_js_a": "document.querySelector('a.burger-menu')?.click()",
    "pre_js_b": "document.querySelector('a.burger-menu')?.click()",
    "wait_ms": 1500
  }
}
```

If `pre_js` executes but the page state doesn't change, a warning is included in the result.

### Widget Schema & Validation

| Tool | Description |
|---|---|
| `list_widgets` | List all widgets with bundled schemas |
| `get_widget_schema` | Full schema for a widget type — valid settings, aliases |
| `validate_element` | Validate element JSON with "did you mean?" suggestions |

### CSS Mapping

| Tool | Description |
|---|---|
| `css_to_elementor` | Convert CSS properties to Elementor widget settings JSON |

### Templates

| Tool | Description |
|---|---|
| `create_template` | Create template with auto-activated Theme Builder conditions |
| `update_template` | Update template data, title, or conditions |
| `get_template` | Get template by ID |
| `list_templates` | List all templates |
| `delete_template` | Delete a template |

### WordPress Options

| Tool | Description |
|---|---|
| `get_wp_option` | Read any WordPress option |
| `set_wp_option` | Write any WordPress option |

### Live Editor

| Tool | Description |
|---|---|
| `elementor_editor` | Control Elementor editor via CDP — open, select widget, set setting, save |

### Widget Scaffolding

| Tool | Description |
|---|---|
| `install_bridge` | Install the MCP Bridge companion plugin |
| `create_widget` | Scaffold and deploy custom Elementor widget PHP class |

### Environment & Setup

| Tool | Description |
|---|---|
| `ensure_site` | Check URL reachability, auto-detect and boot DDEV/Lando environments |
| `setup_wizard` | Interactive WordPress connection setup with options and instructions |

### Utilities

| Tool | Description |
|---|---|
| `seed_content` | Create demo pages with various layouts and widgets |
| `authenticate` | Browser-based WordPress authentication flow |

## Local Environment Detection

The `ensure_site` tool auto-detects local development environments from URLs:

| Pattern | Environment | Action |
|---|---|---|
| `*.ddev.site` | DDEV | Check `ddev list`, start if stopped, verify HTTP |
| `*.lndo.site` | Lando | Check `lando list`, report status |
| `localhost:*`, `127.0.0.1:*` | Local | HTTP reachability check |
| Everything else | Remote | HTTP reachability check |

CDP tools automatically check reachability before navigation. If a local URL is unreachable, the error suggests calling `ensure_site` first.

## MCP Bridge Plugin

Some tools require a small companion WordPress plugin: **MCP Bridge for Page Builders** (`mcp-bridge-for-page-builders` on wordpress.org).

The plugin exposes additional REST endpoints that WordPress's built-in API doesn't provide:

- `elementor-mcp/v1/status` — health check and version probe used by `install_bridge`
- `elementor-mcp/v1/option` — read/write arbitrary WordPress options (used by `get_wp_option`, `set_wp_option`, and as a fallback deployment channel for mu-plugin snippets)

Without the bridge, `get_wp_option` and `set_wp_option` fall back to the standard `wp/v2/settings` endpoint, which only exposes a small allowlisted subset of options. The bridge removes that restriction, giving the MCP server full access to any option — including Elementor's internal configuration, Theme Builder conditions, and third-party plugin settings.

The `install_bridge` tool handles installation automatically using a four-step fallback chain:

1. Check if the bridge is already active (no-op if so)
2. Auto-install and activate from wordpress.org via the plugins REST API
3. Deploy as an mu-plugin snippet via the option endpoint (if step 2 fails)
4. Return the PHP snippet and WP-CLI command for manual installation

The bridge plugin is intentionally minimal — it adds no admin UI, no settings page, and no frontend output. Its sole purpose is to extend the REST API surface available to this MCP server.

## Development

### Running Tests

```bash
# Full cycle: spins up Docker WordPress, runs unit + integration tests, tears down
./tests/run.sh

# Unit tests only (no Docker needed)
./tests/run.sh --unit

# Keep environment running after tests
./tests/run.sh --keep

# Re-run tests against existing environment
./tests/run.sh --retest
```

### Project Structure

```
src/
├── main.rs            — stdio loop, config, tool dispatch (graceful startup)
├── mcp.rs             — JSON-RPC 2.0 protocol + stdio transport
├── wp.rs              — WordPress HTTP client (reqwest + Basic Auth + unconfigured mode)
├── cdp.rs             — Chrome DevTools Protocol (crash recovery, pre_js, local URL check)
├── elementor/         — Element types, tree operations, service layer
└── tools/
    ├── mod.rs         — Tool trait + registry (59 tools)
    ├── page/          — Page CRUD
    ├── post/          — Post CRUD
    ├── element/       — Element operations
    ├── global/        — Global colors/typography
    ├── settings/      — Kit schema, experiments
    ├── visual/        — Screenshots, comparison, diff, match_styles
    ├── schema/        — Widget schemas, validation
    ├── template/      — Theme Builder templates
    ├── file_io/       — Download, upload, backup
    ├── option/        — WordPress options
    ├── bridge/        — Bridge plugin + widget scaffolding
    ├── css_map.rs     — CSS-to-Elementor mapping
    ├── inspect.rs     — DOM inspection
    ├── clone.rs       — DOM-to-Elementor cloning
    ├── editor.rs      — Live Elementor editor control
    ├── ensure_site.rs — Local environment detection + boot
    ├── setup_wizard.rs — Interactive WP connection setup
    ├── seed.rs        — Demo content generation
    ├── auth.rs        — Browser authentication
    └── cache.rs       — CSS cache clearing
```

## License

Licensed under the [Business Source License 1.1](LICENSE).

- Free to use, modify, and distribute for any purpose including production
- Cannot be sold or rebranded as a separate commercial product without a license
- Change Date: 2030-03-26 — converts to Apache 2.0
