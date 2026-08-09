# Brain Index

_Auto-generated. Last updated 2026-08-09T20:56:56.738Z._

- [chrome-cdp-pid-scoped-profiles](pages/chrome-cdp-pid-scoped-profiles.md) — category: decision | **Bug that was fixed (per commit `08404c3 fix: PID-scope Chrome CDP profile dir to eliminate concurrent-instance race`):** earlier, all serv
- [install-config-real-writes-vs-printed-snippets](pages/install-config-real-writes-vs-printed-snippets.md) — category: reference | `install_config` detects the calling host (`detect_host()`) via env vars and config-file presence, then behaves differently per host — **onl
- [mcp-bridge-plugin-purpose](pages/mcp-bridge-plugin-purpose.md) — category: concept | Per README (not independently verified against `install_bridge.rs`'s actual implementation during this pass): WordPress core's REST API only
- [multi-site-credential-store-design](pages/multi-site-credential-store-design.md) — category: decision | Verified by reading `wp.rs` and `main.rs` directly (not just README):
- [panic-isolation-per-tool-call](pages/panic-isolation-per-tool-call.md) — category: decision | Every tool call in the main request-handling loop is wrapped in `catch_unwind` — if a tool's `run()` implementation panics (a Rust panic, no
