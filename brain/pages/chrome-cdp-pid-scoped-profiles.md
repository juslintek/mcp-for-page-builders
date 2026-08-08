---
id: chrome-cdp-pid-scoped-profiles
title: Chrome CDP profile dirs are PID-scoped to prevent concurrent-instance races
category: decision
status: active
created: "2026-07-23T13:54:15"
updated: "2026-07-23T13:54:58"
---

## compiled_truth

**Bug that was fixed (per commit `08404c3 fix: PID-scope Chrome CDP profile dir to eliminate concurrent-instance race`):** earlier, all server instances shared one fixed Chrome profile directory. When one instance's cleanup logic ran, it could `pkill` another concurrently-running instance's live Chrome process — a real race condition affecting anyone running multiple MCP client sessions against this server at once.

**Fix, verified in `cdp.rs`:** each server instance now launches Chrome into a profile directory scoped by its own PID (`mcp-for-page-builders-cdp-<pid>`). On launch, stale profile directories from dead PIDs are swept — but only after checking process liveness via `kill -0`, so a live instance's directory is never touched by a different instance's cleanup pass.

**Why this matters going forward:** if debugging a "Chrome won't launch" or "screenshot tool hangs" issue, check for orphaned profile directories under the temp dir first — the sweep logic assumes `kill -0` reliably detects liveness, which could misbehave across a reboot if PIDs get reused before the sweep runs (not something this brain-bootstrap pass tested, just a plausible edge case worth knowing about).


## timeline

- time: 2026-07-23T13:54:15
  kind: decision
  summary: "Created this page: Chrome CDP profile dirs are PID-scoped to prevent concurrent-instance races"
  source: "most recent 2 git commits + src/cdp.rs"
  affects: [chrome-cdp-pid-scoped-profiles]

- time: 2026-07-23T13:54:58
  kind: decision
  summary: "captured during brain-bootstrap from the two most recent git commits + reading src/cdp.rs"
  source: "git log (08404c3), src/cdp.rs"
  affects: [chrome-cdp-pid-scoped-profiles]
