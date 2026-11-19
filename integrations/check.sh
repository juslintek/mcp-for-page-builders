#!/usr/bin/env sh
set -eu

integrations_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$integrations_dir/.." && pwd)
validator="${CODEX_PLUGIN_VALIDATOR:-$HOME/.codex/skills/.system/plugin-creator/scripts/validate_plugin.py}"

if [ ! -f "$validator" ]; then
  printf '%s\n' "Codex plugin validator not found: $validator" >&2
  exit 1
fi

for manifest in \
  "$repo_root/.agents/plugins/marketplace.json" \
  "$repo_root/plugins/mcp-for-page-builders/.codex-plugin/plugin.json" \
  "$repo_root/plugins/mcp-for-page-builders/.mcp.json" \
  "$integrations_dir/claude-code/.claude-plugin/plugin.json" \
  "$integrations_dir/claude-code/.mcp.json" \
  "$integrations_dir/antigravity/plugin.json" \
  "$integrations_dir/antigravity/mcp_config.json" \
  "$integrations_dir/agent-plugin/plugin.json" \
  "$integrations_dir/agent-plugin/mcp.json" \
  "$integrations_dir/gemini-cli/gemini-extension.json"
do
  jq empty "$manifest"
done

python3 "$validator" "$repo_root/plugins/mcp-for-page-builders"

response=$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"package-check","version":"0.0.0"}}}' | "$repo_root/plugins/mcp-for-page-builders/bin/mcp-for-page-builders")
printf '%s\n' "$response" | jq -e '
  .jsonrpc == "2.0" and
  .id == 1 and
  .result.protocolVersion == "2024-11-05" and
  .result.serverInfo.name == "mcp-for-page-builders" and
  .result.serverInfo.version == "0.2.0"
' >/dev/null
