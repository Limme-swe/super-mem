# Pi adapter

Pi uses a native extension instead of MCP. Install supermem on PATH, then install the adapter from a checkout:

~~~sh
pi install ./adapters/pi
~~~

The manifest is configured for @super-mem/pi, but the package is not published.

## Behavior

- before_agent_start records the prompt and recalls context.
- message_end buffers the finalized assistant message.
- agent_settled writes one partial checkpoint.
- session_compact stores the native compaction summary.
- /super-mem-status checks the integration without adding model tools.

Repository and workspace scope control recall across Pi sessions. Session IDs are retained as provenance.

Automatic prompt, assistant, and compaction text is capped at 64 KiB before it crosses stdin. Every subprocess call is bounded and fail-open. Use the supermem CLI for explicit memory management.
