---
id: panic-isolation-per-tool-call
title: "Every tool call is wrapped in catch_unwind so one bad call can't kill the server"
category: decision
status: active
created: "2026-07-23T13:54:15"
updated: "2026-07-23T13:54:58"
---

## compiled_truth

Every tool call in the main request-handling loop is wrapped in `catch_unwind` — if a tool's `run()` implementation panics (a Rust panic, not a recoverable `Result::Err`), the panic is caught at the dispatch boundary and turned into an error response, rather than unwinding all the way up and killing the whole server process.

**Why this matters:** an MCP server that dies mid-session is much worse for a client than a single tool call returning an error — the client has no way to distinguish "no response because crashed" from "slow" without a timeout, and any other in-flight or subsequent tool calls in the same session would also fail. This is a deliberate resilience choice, not a default Rust behavior (Rust panics unwind and can abort a process by default unless explicitly caught).

Combined with the most recent commit's stdin-robustness fix (blank/malformed input lines no longer silently kill the server) and the PID-scoped Chrome profile fix (see [[chrome-cdp-pid-scoped-profiles]]), this reads as a consistent pattern: this project treats "the server process must stay alive across a whole session" as a hard requirement, and recent commits are actively closing gaps in that guarantee. If adding a new tool, its `run()` should not rely on the caller to prevent panics — but also shouldn't panic deliberately as a control-flow mechanism, since the isolation is a safety net, not an intended error-reporting path.


## timeline

- time: 2026-07-23T13:54:15
  kind: decision
  summary: "Created this page: Every tool call is wrapped in catch_unwind so one bad call can't kill the server"
  source: "src/main.rs, read directly"
  affects: [panic-isolation-per-tool-call]

- time: 2026-07-23T13:54:58
  kind: decision
  summary: captured during brain-bootstrap via direct reading of src/main.rs
  source: "src/main.rs, read directly"
  affects: [panic-isolation-per-tool-call]
