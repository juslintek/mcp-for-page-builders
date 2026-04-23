---
name: mcp-for-page-builders
description: "MCP server for WordPress — 71 tools: multi-site management, full WP REST API, page/post CRUD, Elementor element ops, media upload, global design tokens, visual comparison, widget validation, CDP DOM inspection, CSS-to-Elementor mapping, live editor control, DOM cloning, widget scaffolding."
---

# mcp-for-page-builders MCP Server

High-performance MCP server for WordPress + Elementor. 71 tools covering multi-site credential management, full WordPress REST API access, page/post/template CRUD, element-level operations, media upload, global design tokens, visual comparison, style extraction, widget schema validation, CDP DOM inspection, CSS-to-Elementor mapping, live editor control, DOM cloning, and widget scaffolding.

## When to Load

When the user mentions:
- WordPress sites, pages, posts, media, users, comments, categories, tags
- Elementor, widgets, containers, page builder
- Connecting to or managing WordPress sites
- Uploading files/media to WordPress
- WordPress REST API
- Visual comparison, screenshots, style extraction
- MCP for page builders

## Setup

### Binary
```bash
git clone https://github.com/juslintek/mcp-for-page-builders
cd mcp-for-page-builders && cargo build --release
# Binary: target/release/mcp-for-page-builders
```

### Client Config (Kiro / Claude Desktop / Cursor / etc.)
```json
{
  "mcpServers": {
    "mcp-for-page-builders": {
      "command": "/path/to/mcp-for-page-builders"
    }
  }
}
```

**No credentials needed in config.** The server manages its own credential storage internally at `~/.config/mcp-for-page-builders/sites.json`. Use `connect_site` or `authenticate` tools to add sites at runtime.

**Legacy env-var mode** still works — set `WP_URL`, `WP_APP_USER`, `WP_APP_PASSWORD` in the `env` block if preferred.

## Credential Storage

Credentials are stored internally by the MCP server — **never in the MCP client config**.

- Storage file: `~/.config/mcp-for-page-builders/sites.json`
- Supports multiple sites with an "active" site concept
- Auto-loads on startup, auto-saves on connect/disconnect/switch
- Use `list_sites` to see all connections
- Use `connect_site` to add, `disconnect_site` to remove, `switch_site` to change active

## Available Tools (71)

### Site Management (4)
| Tool | Description |
|---|---|
| `list_sites` | List all stored WordPress site connections (shows active site) |
| `connect_site` | Add a WordPress site (url, user, app_password) — saved to internal storage |
| `disconnect_site` | Remove a stored site connection |
| `switch_site` | Change the active WordPress site |

### WordPress REST API (8)
| Tool | Description |
|---|---|
| `wp_api` | Call ANY WP REST endpoint — method (GET/POST/PUT/DELETE/PATCH), endpoint, body, query |
| `list_users` | List WordPress users with search/pagination |
| `list_comments` | List WordPress comments with search/pagination |
| `list_categories` | List WordPress categories |
| `list_tags` | List WordPress tags |
| `list_media` | List WordPress media items |
| `wp_search` | Search across all WordPress content types |
| `upload_media` | Upload file to media library (local file_path or base64 file_data + filename) |

### Content Management (11)
| Tool | Description |
|---|---|
| `create_page` | Create page with Elementor data |
| `get_page` | Get page by ID with `_elementor_data` |
| `update_page` | Update page (auto-clears CSS cache) |
| `delete_page` | Delete page |
| `get_page_by_slug` | Look up page ID from URL slug |
| `list_pages` | List pages with filters |
| `create_post` | Create WordPress post |
| `get_post` | Get post by ID |
| `list_posts` | List posts with search/filter |
| `update_post` | Update post |
| `delete_post` | Delete post |

### Element Operations (8)
| Tool | Description |
|---|---|
| `get_element_tree` | Flattened view of page structure with paths and IDs |
| `get_element` | Get single element by ID |
| `add_element` | Insert widget/container at position |
| `update_element` | Partial settings merge |
| `remove_element` | Delete element by ID |
| `move_element` | Move to different parent/position |
| `duplicate_element` | Clone with new IDs |
| `find_elements` | Search by widget type or setting |

### Templates (5)
| Tool | Description |
|---|---|
| `create_template` | Create template with auto-activated Theme Builder conditions |
| `update_template` | Update template data, title, or conditions |
| `get_template` | Get template by ID |
| `list_templates` | List all templates |
| `delete_template` | Delete a template |

### Global Design (6)
| Tool | Description |
|---|---|
| `get_global_colors` | Get all global color tokens |
| `set_global_color` | Create/update a global color |
| `delete_global_color` | Delete a global color |
| `get_global_typography` | Get all typography presets |
| `set_global_typography` | Create/update a typography preset |
| `delete_global_typography` | Delete a typography preset |

### Settings & Kit (4)
| Tool | Description |
|---|---|
| `get_kit_schema` | All available kit settings with types and defaults |
| `get_kit_defaults` | Default widget settings |
| `get_experiments` | Feature flags and their state |
| `set_experiment` | Toggle experiments |

### WordPress Options (2)
| Tool | Description |
|---|---|
| `get_wp_option` | Read any WordPress option |
| `set_wp_option` | Write any WordPress option |

### Visual & CDP (8)
| Tool | Description |
|---|---|
| `screenshot` | Full-page screenshot of any URL |
| `screenshot_page` | Screenshot WordPress page by ID |
| `visual_compare` | Side-by-side comparison of two URLs |
| `visual_diff` | Element-by-element comparison via CDP |
| `extract_styles` | Extract computed CSS from live page element |
| `match_styles` | One-shot: extract → convert → apply styles to Elementor element |
| `inspect_page` | DOM inspection via CDP — bounding box, styles, children |
| `clone_element` | Clone live DOM element as Elementor JSON |

### CSS & Schema (4)
| Tool | Description |
|---|---|
| `css_to_elementor` | Convert CSS properties to Elementor settings JSON |
| `list_widgets` | List all widgets with bundled schemas |
| `get_widget_schema` | Full schema — valid settings, common mistakes |
| `validate_element` | Validate JSON with "did you mean?" suggestions |

### File I/O (3)
| Tool | Description |
|---|---|
| `download_page` | Save Elementor data to local JSON file |
| `upload_page` | Update page from local JSON file |
| `backup_page` | Snapshot current state to timestamped file |

### Bridge & Scaffolding (2)
| Tool | Description |
|---|---|
| `install_bridge` | Install MCP Bridge plugin |
| `create_widget` | Scaffold and deploy custom Elementor widget PHP class |

### Utilities (6)
| Tool | Description |
|---|---|
| `seed_content` | Create demo pages with various layouts |
| `authenticate` | Browser-based WordPress authentication flow |
| `setup_wizard` | Show setup instructions when not configured |
| `ensure_site` | Check URL reachability, boot local dev environments |
| `clear_cache` | Clear Elementor CSS cache |
| `elementor_editor` | Control Elementor editor via CDP |

## Key Workflows

### Connect to a WordPress site (no config needed)
```
connect_site(url: "https://my-site.com", user: "admin", app_password: "xxxx xxxx xxxx")
```
Or use `authenticate` for browser-based approval flow.

### Manage multiple sites
```
connect_site(url: "https://site-a.com", ...)
connect_site(url: "https://site-b.com", ...)
list_sites()                              → shows all sites, marks active
switch_site(url: "https://site-b.com")    → changes active site
```

### Call any WordPress REST API endpoint
```
wp_api(method: "GET", endpoint: "wp/v2/plugins")
wp_api(method: "POST", endpoint: "wp/v2/posts", body: {"title": "Hello", "status": "draft"})
wp_api(method: "PUT", endpoint: "wp/v2/posts/123", body: {"title": "Updated"})
wp_api(method: "DELETE", endpoint: "wp/v2/posts/123")
```

### Upload media
```
upload_media(file_path: "/path/to/image.jpg", title: "Hero Image", alt_text: "Banner")
upload_media(file_data: "<base64>", filename: "logo.png")
```

### Build a page from scratch
1. `create_page` with Elementor JSON
2. `get_element_tree` to verify structure
3. `update_element` to tweak settings
4. `screenshot_page` to verify visually

### Clone a legacy design
```
inspect_page(url: "https://old-site.com", selector: "header")
clone_element(url: "https://old-site.com", selector: "header")
visual_diff(url_a: "https://old-site.com", url_b: "https://new-site.com")
```

### Validate before writing
```
get_widget_schema(widget_type: "heading")
validate_element(element: {...})           → "did you mean?" suggestions
```

## CDP Session
- Persistent Chrome session launched lazily on first visual/CDP tool call
- Set `CHROME_PATH` env var to override Chrome location
- Session shared across all tool calls for the MCP server lifetime

## Tools That Work Without WordPress
These tools work in CDP-only mode (no WordPress connection needed):
- `screenshot`, `visual_compare`, `visual_diff`, `inspect_page`
- `extract_styles`, `match_styles`, `clone_element`
- `css_to_elementor`, `list_widgets`, `get_widget_schema`, `validate_element`
- `ensure_site`
