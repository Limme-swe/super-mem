# Pi adapter

Pi uses a native extension instead of MCP. Install supermem on PATH, then install the adapter from a checkout:

~~~sh
pi install ./adapters/pi
~~~

The manifest is configured for @super-mem/pi, but the package is not published.

## Behavior

- before_agent_start records the prompt and recalls context.
- message_end buffers the finalized assistant message.
- tool_execution_start and tool_execution_end correlate arguments with results, then record command results, file changes, and other tool outcomes. Test, lint, type-check, and build commands are marked as verification evidence.
- agent_settled writes one partial checkpoint.
- session_compact stores the native compaction summary.
- /super-mem-status checks the integration without adding model tools.

Repository and workspace scope control recall across Pi sessions. Set `SUPER_MEM_NAMESPACE` and `SUPER_MEM_WORKSPACE` in the environment that launches Pi when using a non-default scope. Session IDs are retained as provenance.

Automatic prompt, assistant, and compaction text is capped at 64 KiB before it crosses stdin. Tool commands and results are sent through stdin rather than process arguments; the Rust core bounds and redacts them before storage. Memory tools are excluded from automatic tool capture. Every subprocess call is bounded and fail-open. Use the supermem CLI for explicit memory management.
