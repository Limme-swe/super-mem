# Integration checklist

Use this checklist after installing a Codex, Claude Code, OpenCode, Pi, or generic MCP integration. It distinguishes the explicit MCP path from automatic lifecycle capture and verifies that both use the same hard scope.

## 1. Verify the host environment

Run these from the same shell, service, or application environment that launches the harness:

```sh
supermem --version
python scripts/preflight.py --cwd /absolute/path/to/repository
```

Confirm that:

- the intended `supermem` executable is resolved;
- the configured database parent is writable;
- the repository is readable by Git;
- the working directory is the repository expected by the agent.

Initialize the store once through the normal CLI path:

```sh
supermem init
```

## 2. Choose one hard scope

Record the intended values before editing adapter files:

```text
SUPER_MEM_DB=<default or absolute canonical database path>
SUPER_MEM_NAMESPACE=<stable namespace, default is default>
SUPER_MEM_WORKSPACE=<optional stable workspace>
MCP_ROOT=<absolute repository root>
```

MCP, command hooks, plugins, and extensions must agree. Namespace and workspace are isolation boundaries, not preferences.

## 3. Install the harness adapter

### Codex

Install the included marketplace/plugin from `.agents/plugins/marketplace.json`, or use the manual files under `adapters/codex`. Confirm the MCP server command is `supermem mcp --root .` and that the three scope environment variables are forwarded rather than hard-coded to conflicting values.

### Claude Code

Register MCP for the project:

```sh
claude mcp add --scope project --transport stdio super_mem -- supermem mcp --root .
```

Install and review the hook configuration under `adapters/claude`.

### OpenCode

Install the files under `adapters/opencode`. Confirm the local MCP command starts `supermem mcp --root .` and the TypeScript plugin is enabled.

### Pi

From the extracted release archive or source checkout:

```sh
pi install ./adapters/pi
```

Use `/super-mem-status` inside Pi for the extension-facing status path.

### Generic MCP client

```json
{
  "mcpServers": {
    "super-mem": {
      "command": "supermem",
      "args": ["mcp", "--root", "/absolute/path/to/repository"]
    }
  }
}
```

## 4. Test explicit storage and recall outside the harness

```sh
supermem remember \
  --kind fact \
  --body "Integration verification marker" \
  --canonical-key integration-verification-marker \
  --cwd /absolute/path/to/repository

supermem recall \
  --query "integration verification marker" \
  --cwd /absolute/path/to/repository \
  --format json
```

If this fails, fix installation, scope, database, or Git discovery before debugging the harness.

## 5. Test MCP access

Start a new harness session in the repository and ask it to recall the verification marker through `memory_context`. Then record a second harmless marker through `memory_record` and confirm it from the CLI.

The model-facing tools should be limited to:

- `memory_context`;
- `memory_record`;
- `memory_feedback`;
- `memory_manage`.

Database status, import/export, and physical purge remain CLI-only operations.

## 6. Test automatic capture

Perform a harmless command or file operation in the harness, finish the turn, then recall a distinctive non-sensitive phrase from that event or checkpoint.

Expected behavior differs by host, but reference adapters are fail-open: the coding session continues when memory is unavailable. Therefore, “the agent did not crash” is not a capture test.

## 7. Run observational diagnostics

```sh
supermem --json doctor --cwd /absolute/path/to/repository
```

Run it with the same environment and executable search path as the harness. Check binary identity, database resolution, schema, writer-lock availability, sidecars, Git discovery, and redacted scope sources.

Create a reviewed support report when needed:

```sh
python scripts/support_bundle.py \
  --cwd /absolute/path/to/repository \
  --output super-mem-support.json
```

## 8. Validate compaction and restart behavior

For a host with compaction support:

1. record a distinctive decision;
2. trigger or wait for compaction;
3. confirm the compact summary is checkpointed where supported;
4. start or resume a session;
5. verify relevant context is recalled without duplicating the final assistant message.

## 9. Remove the test marker

Inspect it first, then retract it so history remains attributable:

```sh
supermem --json inspect MEMORY_ID --history
supermem retract MEMORY_ID --reason "Integration verification completed"
```

## Failure isolation

| Result | Likely layer |
| --- | --- |
| CLI write and recall fail | Installation, database, scope, or Git discovery. |
| CLI works; MCP tools absent | Harness MCP configuration or launch path. |
| MCP works; lifecycle events absent | Hook, plugin, or extension configuration. |
| Records exist but do not recall | Scope mismatch, Git applicability, lifecycle state, or query. |
| Only one harness fails | Host-specific adapter installation or event contract. |

The complete host-specific behavior is documented in [Harness integrations](integrations.md). Use [Troubleshooting](troubleshooting.md) before changing core retrieval settings.
