# Client integrations

Every package starts the same Cargo-installed stdio server. Install it once from this checkout:

```bash
cargo install --path /Volumes/Projects/mcp-for-page-builders
```

From another checkout, replace that path with the path to its source. The packages do not bundle a release binary.

## Codex

```bash
codex plugin marketplace add /Volumes/Projects/mcp-for-page-builders
codex plugin add mcp-for-page-builders@personal
```

## Claude Code

Load the local package for a session:

```bash
claude --plugin-dir /Volumes/Projects/mcp-for-page-builders/integrations/claude-code
```

Claude Code needs a marketplace for a persistent install; this wrapper deliberately does not add one.

## Antigravity

```bash
agy plugin install /Volumes/Projects/mcp-for-page-builders/integrations/antigravity
```

Antigravity runs `mcp-for-page-builders` directly, so Cargo's bin directory must be on its `PATH`.

## Agent Plugins 1.0

This package is for Cursor, VS Code, GitHub Copilot CLI, and GitHub Copilot in VS Code.

```bash
copilot plugin install /Volumes/Projects/mcp-for-page-builders/integrations/agent-plugin
```

For VS Code, register the local directory in `settings.json`:

```json
{
  "chat.pluginLocations": {
    "/Volumes/Projects/mcp-for-page-builders/integrations/agent-plugin": true
  }
}
```

For Cursor, link it and reload the window:

```bash
ln -s /Volumes/Projects/mcp-for-page-builders/integrations/agent-plugin ~/.cursor/plugins/local/mcp-for-page-builders
```

## Gemini CLI

```bash
gemini extensions install /Volumes/Projects/mcp-for-page-builders/integrations/gemini-cli --consent
```

Use `gemini extensions link` with the same path while developing the wrapper.

Credentials remain in the server's existing configuration or environment. Do not put them in these manifests.
