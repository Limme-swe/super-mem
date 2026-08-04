# Codex adapter

Install the `supermem` binary first, then install this directory as a Codex
plugin. Codex discovers `hooks/hooks.json` automatically; the MCP server is
declared directly in the plugin manifest. Review and trust the hook definition
when Codex prompts. For a configuration-only install, copy `hooks/hooks.json`
to `.codex/hooks.json` and merge `config.toml` into the active Codex
configuration. Verify hooks with `/hooks` and MCP with `codex mcp list` or
`/mcp verbose`.

The example MCP launch pins `--root . --namespace default`. Replace `.` with an
absolute trusted repository root when the host process working directory is not
guaranteed. Add `--workspace <id>` when workspace isolation is required; these
hard boundaries are never accepted from model tool arguments.

The hook consumes the official JSON payload on stdin. It observes prompts and
uses each final message to create one partial checkpoint, without a duplicate
assistant observation. Automatic text is capped at 64 KiB. `transcript_path`
is retained as provenance only because Codex documents the transcript
representation as unstable.

The current hook launches the CLI synchronously and fails open. A future local
daemon will be hidden behind `supermem hook codex`, so this configuration will
not need to change.
