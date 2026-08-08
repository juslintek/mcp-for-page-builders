# Known issue: screenshot / screenshot_page against a session-authenticated, non-WordPress admin panel

Found 2026-07-28 while verifying an EasyAdmin (Symfony) admin panel change on `loyalty-hub`, a
non-WordPress project. Not yet migrated into the project brain — the `brain` CLI was not available in
that session (no `brain.mjs` found under any installed skill path, no `brain` binary on PATH). Whoever
picks this up with brain access should read this note, verify it still applies, and record it as a
proper decision/issue page via `brain append-timeline` or equivalent, then delete this file.

## What was tried

`screenshot` (generic, URL-based) against `http://localhost:96/admin/tier-policy-rule` — an EasyAdmin
page that requires a logged-in session (form login, session cookie, no WordPress involved at all).

`screenshot_page` was tried first and is a **hard mismatch, not a bug**: its implementation
(`src/tools/visual/screenshot_page.rs`) calls `wp.get("wp/v2/pages/{page_id}")` unconditionally — it is
built entirely around the WordPress REST API and has no meaning for a non-WordPress app. This is
correct as designed; the mistake was reaching for a WP-specific tool outside its scope. Not a defect.

## The real gap: no first-class way to hand cdp_screenshot an authenticated session

`screenshot` and `cdp_screenshot` (`src/tools/visual/mod.rs`, `src/cdp.rs`) have no `cookies` or
`storage_state` parameter. The only way to reach an authenticated page is:

1. Rely on the Chrome profile persisting across calls **within the same running MCP server process**
   (`user_data_dir` is scoped to `std::process::id()` in `src/cdp.rs`, so it *is* stable for the life of
   one server instance — this is not itself broken), and
2. Use `pre_js` to drive a real form login on a prior call, so the session cookie lands in that same
   profile before a later screenshot call reuses it.

Neither of these is documented anywhere in the tool descriptions or README. In practice this means:
first-time use against any session-authenticated app looks broken (screenshot only ever shows the
login page) until the caller works out, undocumented, that they need a separate `pre_js`-driven login
call first, using the *same* running server instance. A caller who restarts the MCP server between the
login call and the screenshot call gets a fresh empty profile and silently loses the session with no
error explaining why.

Suggested fix directions (not yet decided, needs a maintainer call):
- Add a `cookies` (or `storage_state`) parameter to `screenshot`/`screenshot_page` so a caller can pass
  a session cookie directly, without needing `pre_js` to reproduce a full login flow.
- At minimum, document the current `pre_js`-login-then-screenshot pattern and the per-process-PID
  profile lifetime in the tool descriptions and README, so it is not tribal knowledge.
- Consider whether `pre_js` login should support waiting for a real navigation/redirect before the
  screenshot call captures state, since login is itself a POST + redirect.

## What was used instead in that session

PinchTab (`pinchtab_navigate` / `pinchtab_fill` / `pinchtab_keyboard_type` / `pinchtab_click` with
`waitNav: true`, then `pinchtab_get_text` / `pinchtab_snapshot`) — a real, persistent browser session
that keeps cookies naturally across calls the same way a human's browser would, so no special handling
was needed. This is the recommended fallback for any session-authenticated, non-WordPress target until
the gap above is addressed.

## Also worth confirming still holds

`src/tools/visual/mod.rs` documents a **previously real, now-fixed** issue in its own comments — worth
double-checking it doesn't regress: full-page PNG screenshots could exceed some MCP clients' JSON-RPC
line-read buffer (documented example: Kiro CLI's ~1 MB line buffer), surfacing as "Transport closed /
non-JSON-RPC output" errors. Fixed via a `MAX_INLINE_JPEG_BYTES` budget (700 KB) with progressive JPEG
re-encoding — the PNG on disk is unaffected, only the inline image is downsized. No evidence this session
that it has regressed; noted here only because it is the same class of "silently looks broken with no
clear error" failure as the auth gap above, and worth an eye if oversized-image symptoms reappear.
