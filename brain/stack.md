---
slug: stack
title: Tech stack
role: tech-stack choices
updated: "2026-07-23T13:53:25"
---

# Tech stack

| Domain | Choice | Rationale / evidence |
|---|---|---|
| Language | Rust, edition 2024 | `Cargo.toml` |
| License | BSL-1.1 (Business Source License) | Not MIT/Apache — a source-available-but-restricted license, worth knowing before assuming permissive open-source terms |
| Async runtime | Tokio (multi-thread rt, io, fs, process features) | `Cargo.toml` |
| HTTP client | `reqwest` with `rustls-tls` (NOT native-tls), json + multipart features | Deliberate TLS backend choice — avoids linking OpenSSL |
| Browser automation | `chromiumoxide` 0.9 (CDP) | Powers all visual/screenshot/clone_element/inspect tools |
| Serialization | `serde` / `serde_json` | Standard |
| TLS/cert generation | `rcgen` + `tokio-rustls` + `rustls-pemfile` | Present in deps; likely used for a local HTTPS callback during the browser-based `authenticate` flow — not independently confirmed against `auth.rs` during this pass |
| Logging | `tracing` + `tracing-subscriber` + `tracing-appender` | File logging added in the most recent commit alongside the stdin-robustness fix |
| Error handling | `anyhow` | |
| Async trait | `async-trait` | Used for the `Tool` trait's async `run()` |
| Linting | `unsafe_code = "forbid"`, clippy all+pedantic+nursery warn, OOP-favoring restriction lints (e.g. `use_self`) | Deliberately strict — see commit `e21ee0c` |
| Config/credential storage | Plain JSON file at `~/.config/mcp-for-page-builders/sites.json` | See [[multi-site-credential-store-design]] |
| Companion component | A separate WordPress plugin ("MCP Bridge") | Not part of this Rust codebase — installed into WordPress separately, see [[mcp-bridge-plugin-purpose]] |
| Local dev env support | DDEV / Lando auto-detection and boot (`ensure_site` tool) | For working against a local WordPress instance rather than only production |
| Testing | `tests/` directory present + `docker-compose.test.yml` | Docker-based test environment; contents not read in full during this pass |

## Distribution

5.4MB binary, ~3MB RAM, ~1ms startup (README's own performance claims — not independently benchmarked during this brain-bootstrap pass, but plausible for a native Rust binary with lazy Chrome launch).
