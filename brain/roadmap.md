---
slug: roadmap
title: Roadmap
role: milestones
updated: "2026-07-23T13:54:06"
---

# Roadmap

No dated milestones in the docs read — progress is tracked entirely via git log (15 commits), reflecting a young, actively-iterated project.

## Evolution visible from git log (oldest to newest)

1. Initial README, then full Rust source added.
2. MCP Bridge plugin docs added — explaining the companion WordPress plugin's purpose.
3. Repository URL cleanup, `.idea` untracked.
4. **Major refactor**: extracted args helpers, `ElementTree`/`ElementorService`, `SchemaRegistry` — then split modules into single-struct-per-file structure.
5. Strict clippy ruleset adopted (all+pedantic+nursery, OOP-favoring restriction lints) — a deliberate code-quality tightening pass.
6. **Feature expansion**: internal credential storage + multi-site support + full WP REST API + file upload — the multi-site architecture described in [[multi-site-credential-store-design]] landed here.
7. `install_config` self-installing skill/agent/prompt config, then extended to support all major LLM CLI agents (with the caveat in [[install-config-real-writes-vs-printed-snippets]] about which ones get real file writes vs. printed snippets).
8. `patch_elements` tool + session continuity (lockfile + journal + `get_session_state`) — bulk-update efficiency + crash/orphan recovery.
9. Auth flow cleanup — password collection step removed (presumably in favor of the Application Password browser-approval flow).
10. Reliability fixes (most recent 2 commits): PID-scoped Chrome CDP profile dir (race-condition fix for concurrent instances), then stdin robustness + file logging (prevents silent server death on malformed input).

## Reading this trajectory

The project has moved from "get core Elementor/WP tools working" → "refactor for maintainability + strict lints" → "expand scope (multi-site, full REST API, cross-agent installer)" → "harden reliability for real concurrent/long-running use" — a fairly mature progression for 15 commits, suggesting either a compressed intense development period or a well-planned build-out. No open TODOs or known-issues doc was found during this pass; if there's a backlog, it likely lives outside this repo (e.g. an issue tracker not visible from the filesystem).
