import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

// The Rust engine waits up to five seconds for a contended SQLite writer.
// Keep the host deadline above that bound so fail-open handling can complete.
const TIMEOUT_MS = 8000
const MAX_CAPTURE_BYTES = 64 * 1024
const CAPTURE_TRUNCATION_MARKER =
  "\n… [super-mem automatic capture truncated; middle omitted] …\n"

function automaticIdempotencyKey(
  domain: string,
  fields: readonly string[],
): string {
  const hash = createHash("sha256")
  hash.update("super-mem automatic idempotency key derivation v1\0", "utf8")
  const updateFrame = (value: string): void => {
    hash.update(String(Buffer.byteLength(value, "utf8")), "ascii")
    hash.update(":", "ascii")
    hash.update(value, "utf8")
  }
  updateFrame(domain)
  updateFrame(String(fields.length))
  for (const field of fields) updateFrame(field)
  return `sm1:${hash.digest("hex")}`
}

function capCapture(value: string): string {
  if (Buffer.byteLength(value, "utf8") <= MAX_CAPTURE_BYTES) return value
  const contentBudget =
    MAX_CAPTURE_BYTES - Buffer.byteLength(CAPTURE_TRUNCATION_MARKER, "utf8")
  const headBudget = Math.floor(contentBudget / 2)
  const tailBudget = contentBudget - headBudget
  let head = ""
  let used = 0
  for (const symbol of value) {
    const size = Buffer.byteLength(symbol, "utf8")
    if (used + size > headBudget) break
    head += symbol
    used += size
  }
  const tail: string[] = []
  used = 0
  for (let end = value.length; end > 0; ) {
    let start = end - 1
    const trailing = value.charCodeAt(start)
    if (trailing >= 0xdc00 && trailing <= 0xdfff && start > 0) {
      const leading = value.charCodeAt(start - 1)
      if (leading >= 0xd800 && leading <= 0xdbff) start -= 1
    }
    const symbol = value.slice(start, end)
    const size = Buffer.byteLength(symbol, "utf8")
    if (used + size > tailBudget) break
    tail.push(symbol)
    used += size
    end = start
  }
  return `${head}${CAPTURE_TRUNCATION_MARKER}${tail.reverse().join("")}`
}

function run(args: string[], input?: string): string | undefined {
  try {
    const result = spawnSync("supermem", args, {
      encoding: "utf8",
      input,
      timeout: TIMEOUT_MS,
      stdio: [input === undefined ? "ignore" : "pipe", "pipe", "ignore"],
    })
    if (result.status !== 0 || result.error) return undefined
    return result.stdout.trim() || undefined
  } catch {
    return undefined
  }
}

type ToolEventKind = "command_result" | "file_change" | "tool_result"

interface PendingTool {
  toolName: string
  args: unknown
}

function objectOf(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return undefined
  }
  return value as Record<string, unknown>
}

function serialize(value: unknown): string {
  if (typeof value === "string") return value
  if (value === undefined) return ""
  const seen = new WeakSet<object>()
  try {
    const encoded = JSON.stringify(value, (_key, nested: unknown) => {
      if (typeof nested === "bigint") return nested.toString()
      if (typeof nested === "object" && nested !== null) {
        if (seen.has(nested)) return "[Circular]"
        seen.add(nested)
      }
      return nested
    })
    return encoded ?? String(value)
  } catch {
    return String(value)
  }
}

function isSuperMemTool(toolName: string): boolean {
  const normalized = toolName.toLowerCase()
  return (
    normalized === "supermem" ||
    normalized.startsWith("supermem__") ||
    normalized.includes("__supermem__") ||
    normalized.includes("super_mem") ||
    normalized.includes("super-mem") ||
    [
      "memory_context",
      "memory_record",
      "memory_feedback",
      "memory_manage",
    ].includes(normalized)
  )
}

function leafToolName(toolName: string): string {
  return toolName.toLowerCase().split(/__|[.:/]/).at(-1) ?? ""
}

function toolEventKind(toolName: string): ToolEventKind {
  const leaf = leafToolName(toolName)
  if (["bash", "shell", "exec", "exec_command", "terminal"].includes(leaf)) {
    return "command_result"
  }
  if (
    ["edit", "write", "apply_patch", "multiedit", "notebookedit"].some(
      (name) => leaf === name || leaf.includes(name),
    )
  ) {
    return "file_change"
  }
  return "tool_result"
}

function commandOf(args: unknown): string | undefined {
  const input = objectOf(args)
  if (!input) return undefined
  for (const key of ["command", "cmd", "script"]) {
    const value = input[key]
    if (typeof value === "string" && value.trim()) return value
  }
  return undefined
}

function isVerificationCommand(command: string | undefined): boolean {
  if (!command) return false
  return /(?:^|[;&|]\s*|\s)(?:cargo\s+(?:test|nextest|check|clippy|build)\b|(?:npm|pnpm|yarn|bun)\s+(?:(?:run|exec)\s+)?(?:test|lint|typecheck|check|build)\b|(?:python(?:3)?\s+-m\s+)?pytest\b|go\s+(?:test|vet|build)\b|(?:dotnet|mvn|gradle|\.\/gradlew)\s+(?:test|check|verify|build)\b|(?:tsc|eslint|biome|ruff|mypy)\b)/i.test(
    command,
  )
}

function toolEvidence(
  toolName: string,
  args: unknown,
  result: unknown,
  succeeded: boolean,
): string {
  const command = commandOf(args)
  const parts = [`Tool: ${toolName}`, `Succeeded: ${String(succeeded)}`]
  if (command) parts.push(`Command:\n${command}`)
  else if (args !== undefined) parts.push(`Input:\n${serialize(args)}`)
  if (result !== undefined) parts.push(`Result:\n${serialize(result)}`)
  return parts.join("\n")
}

function errorFingerprint(
  toolName: string,
  command: string | undefined,
  content: string,
): string {
  const hash = createHash("sha256")
  hash.update("super-mem tool error fingerprint v1\0", "utf8")
  hash.update(toolName, "utf8")
  hash.update("\0", "utf8")
  hash.update(command ?? "", "utf8")
  hash.update("\0", "utf8")
  hash.update(content, "utf8")
  return `smerr1:${hash.digest("hex")}`
}

function messageText(message: any): string {
  if (typeof message?.content === "string") return message.content
  if (!Array.isArray(message?.content)) return ""
  const text: string[] = []
  for (const part of message.content) {
    if (part?.type !== "text") continue
    const value = String(part.text ?? "")
    if (value) text.push(value)
  }
  return text.join("\n")
}

export default function superMem(pi: ExtensionAPI): void {
  let latestGoal = ""
  let latestAssistant = ""
  const pendingTools = new Map<string, PendingTool>()

  const base = (ctx: any): string[] => [
    "--harness",
    "pi",
    "--session",
    ctx.sessionManager.getSessionId(),
    "--cwd",
    ctx.cwd,
  ]

  pi.on("before_agent_start", async (event, ctx) => {
    latestGoal = capCapture(event.prompt)
    latestAssistant = ""
    const eventId = ctx.sessionManager.getLeafId()
    const args = [
      "recall",
      ...base(ctx),
      "--query-stdin",
      "--observe-prompt",
      "--format",
      "context",
    ]
    if (eventId) args.push("--event-id", eventId)
    const memory = run(args, latestGoal)
    if (!memory) return undefined
    return { systemPrompt: `${event.systemPrompt}\n\n${memory}` }
  })

  pi.on("message_end", async (event) => {
    const role = event.message.role
    if (role !== "assistant") return
    const content = messageText(event.message)
    if (!content) return
    latestAssistant = capCapture(content)
  })

  pi.on("tool_execution_start", async (event, ctx) => {
    try {
      const toolName = String(event.toolName ?? "")
      if (!toolName || isSuperMemTool(toolName)) return
      const key = toolKey(ctx, event.toolCallId)
      pendingTools.set(key, { toolName, args: event.args })
      if (pendingTools.size > 512) {
        const oldest = pendingTools.keys().next().value
        if (oldest !== undefined) pendingTools.delete(oldest)
      }
    } catch {
      // Memory must never block Pi.
    }
  })

  pi.on("tool_execution_end", async (event, ctx) => {
    try {
      const key = toolKey(ctx, event.toolCallId)
      const pending = pendingTools.get(key)
      const toolName = String(event.toolName || pending?.toolName || "")
      if (!toolName || isSuperMemTool(toolName)) return
      const input = pending?.args
      const command = commandOf(input)
      const succeeded = !event.isError
      const content = capCapture(
        toolEvidence(toolName, input, event.result, succeeded),
      )
      const args = [
        "observe",
        ...base(ctx),
        "--kind",
        toolEventKind(toolName),
        "--event-id",
        String(event.toolCallId),
        "--tool-name",
        toolName,
        "--succeeded",
        String(succeeded),
        "--verification",
        String(isVerificationCommand(command)),
        "--content-stdin",
      ]
      if (!succeeded) {
        args.push(
          "--error-fingerprint",
          errorFingerprint(toolName, command, content),
        )
      }
      const stored = run(args, content)
      if (stored) pendingTools.delete(key)
    } catch {
      // Memory must never block Pi.
    }
  })

  pi.on("session_compact", async (event, ctx) => {
    const entry = event.compactionEntry as any
    const summary = capCapture(String(entry.summary ?? ""))
    if (!summary) return
    const sessionID = String(ctx.sessionManager.getSessionId())
    const entryID = entry.id
    run([
      "checkpoint",
      ...base(ctx),
      "--summary-stdin",
      "--idempotency-key",
      automaticIdempotencyKey("pi.session-compaction", [
        sessionID,
        entryID === undefined ? "missing" : "present",
        entryID === undefined ? "" : String(entryID),
        summary,
      ]),
    ], summary)
  })

  pi.on("agent_settled", async (_event, ctx) => {
    checkpointLatest(ctx)
  })

  pi.on("session_shutdown", async (_event, ctx) => {
    checkpointLatest(ctx)
    const prefix = `${String(ctx.sessionManager.getSessionId())}\0`
    for (const key of pendingTools.keys()) {
      if (key.startsWith(prefix)) pendingTools.delete(key)
    }
  })

  pi.registerCommand("super-mem-status", {
    description: "Show Super Mem status for this project",
    handler: async (_args, ctx) => {
      const status = run(["status"])
      ctx.ui.notify(status ?? "Super Mem is unavailable", status ? "info" : "warning")
    },
  })

  function checkpointLatest(ctx: any): void {
    if (!latestAssistant.trim()) return
    const summary = capCapture([
      latestGoal.trim() ? `Goal: ${latestGoal.trim()}` : "Goal: coding task",
      `Assistant outcome: ${latestAssistant.trim()}`,
    ].join("\n"))
    const sessionID = String(ctx.sessionManager.getSessionId())
    const stored = run([
      "checkpoint",
      ...base(ctx),
      "--goal",
      "Complete the current Pi coding turn",
      "--summary-stdin",
      "--idempotency-key",
      automaticIdempotencyKey("pi.agent-checkpoint", [sessionID, summary]),
    ], summary)
    if (!stored) return
    latestGoal = ""
    latestAssistant = ""
  }

  function toolKey(ctx: any, toolCallID: unknown): string {
    return `${String(ctx.sessionManager.getSessionId())}\0${String(toolCallID)}`
  }
}
