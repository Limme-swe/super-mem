# Claude Code adapter

Install the `supermem` binary on `PATH` first. Then copy `.mcp.json` and
`.claude/settings.json` to a project, or merge them with
the equivalent user configuration. The MCP server can also be registered with:

```sh
claude mcp add --scope project --transport stdio super_mem -- \
  supermem mcp --root . --namespace default
```

Verify it with `claude mcp list` and `/mcp`.

The checked-in MCP configuration pins `--root . --namespace default` at server
launch. Prefer an absolute trusted repository root for user-level installs, and
add `--workspace <id>` when needed. MCP tool calls cannot override these hard
scope boundaries.

The hook creates one partial checkpoint from `last_assistant_message` at
`Stop`/`SubagentStop` because the Claude transcript is written asynchronously
and may lag; it does not write a duplicate assistant observation. `PostCompact`
records the supplied compact summary. Automatic text is capped at 64 KiB. All
hook failures are fail-open. The MVP launches the CLI synchronously; a future
daemon transport will remain behind the same `supermem hook claude` command.
