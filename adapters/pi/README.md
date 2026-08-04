# Pi adapter

Install the `supermem` binary on `PATH` first. Pi intentionally does not ship
MCP support, so this package uses its native
extension API. Install a checkout with `pi install ./adapters/pi`, or a
published package with `pi install npm:@super-mem/pi`.

The extension injects recall by replacing the per-turn system prompt; one
`recall --observe-prompt` process captures the prompt and returns context with
one Git discovery. It buffers
the finalized assistant message at `message_end`, and writes one partial
checkpoint at `agent_settled`. It also records native compaction summaries.
Automatic prompt, assistant, and compaction text is capped at 64 KiB before it
crosses stdin. `/super-mem-status` checks the integration without exposing extra
model tools.

Every subprocess call is bounded and fail-open. The MVP launches `supermem`
synchronously; a future daemon can be reached behind the same CLI commands.
