---
slug: flow
title: Key flows
role: key flows
updated: "2026-07-23T13:53:44"
---

# Key flows

---
slug: flow
title: Key flows
role: key flows
updated: "2026-07-23T13:53:36"
---

# Key flows

Two representative flows: a WordPress REST call, and a CDP-based visual tool.

```mermaid
sequenceDiagram
  participant Client as MCP Client
  participant Main as main.rs loop
  participant Store as SiteStore
  participant Wp as WpClient

  Client->>Main: tool call, e.g. get_page
  Main->>Store: read active site credentials
  Main->>Wp: dispatch via Tool::run()
  Wp->>Wp: require_configured() guard
  alt no active site
    Wp-->>Main: helpful setup-instructions error
  else configured
    Wp->>Wp: GET wp/v2/pages/id (Basic Auth)
  end
  Main-->>Client: ToolResult (catch_unwind-protected)
```

```mermaid
sequenceDiagram
  participant Client as MCP Client
  participant Main as main.rs loop
  participant Cdp as CdpClient

  Client->>Main: tool call, e.g. screenshot_page
  Main->>Cdp: dispatch
  alt Chrome not yet launched
    Cdp->>Cdp: sweep stale PID-scoped profile dirs
    Cdp->>Cdp: launch Chrome (PID-scoped profile, 30s timeout)
  end
  Cdp->>Cdp: navigate (20s timeout) + capture
  Cdp-->>Main: image bytes
  Main-->>Client: inline image + saved file path
```

## Site-switching flow (no restart)

```mermaid
sequenceDiagram
  participant Client
  participant Tool as connect_site / switch_site
  participant Store as SiteStore (Arc RwLock)
  participant Main as main.rs event loop
  participant Wp as WpClient

  Client->>Tool: switch_site(url)
  Tool->>Store: mutate active pointer
  Tool->>Main: send ServerNotification::Reconfigure
  Main->>Wp: reconfigure(new creds)
  Wp->>Wp: rebuild reqwest client + auth header in place
  Note over Client,Wp: subsequent tool calls use the new site,<br/>no process restart, no stale-connection error
```
