import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import type { Plugin } from "@opencode-ai/plugin"

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
      input,
      encoding: "utf8",
      timeout: TIMEOUT_MS,
      stdio: [input === undefined ? "ignore" : "pipe", "pipe", "ignore"],
    })
    if (result.status !== 0 || result.error) return undefined
    const value = result.stdout.trim()
    return value || undefined
  } catch {
    return undefined
  }
}

type ToolEventKind = "command_result" | "file_change" | "tool_result"

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

function toolSucceeded(metadata: unknown): boolean {
  const value = objectOf(metadata)
  if (!value) return true
  for (const key of ["isError", "is_error", "failed"]) {
    if (value[key] === true) return false
  }
  for (const key of ["success", "succeeded", "ok"]) {
    if (typeof value[key] === "boolean") return value[key] as boolean
  }
  for (const key of ["exitCode", "exit_code", "exit", "status"]) {
    if (typeof value[key] === "number") return value[key] === 0
  }
  if (
    typeof value.status === "string" &&
    ["error", "failed", "failure"].includes(value.status.toLowerCase())
  ) {
    return false
  }
  return true
}

function toolEvidence(
  toolName: string,
  args: unknown,
  output: { title?: unknown; output?: unknown; metadata?: unknown },
  succeeded: boolean,
): string {
  const command = commandOf(args)
  const parts = [`Tool: ${toolName}`, `Succeeded: ${String(succeeded)}`]
  if (command) parts.push(`Command:\n${command}`)
  else if (args !== undefined) parts.push(`Input:\n${serialize(args)}`)
  if (output.title !== undefined && String(output.title).trim()) {
    parts.push(`Title: ${String(output.title)}`)
  }
  if (output.output !== undefined) {
    parts.push(`Output:\n${serialize(output.output)}`)
  }
  if (output.metadata !== undefined) {
    parts.push(`Metadata:\n${serialize(output.metadata)}`)
  }
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

function textOf(parts: any[]): string {
  const text: string[] = []
  for (const part of parts) {
    if (part?.type !== "text" || part.synthetic) continue
    const value = String(part.text ?? "")
    if (value) text.push(value)
  }
  return text.join("\n")
}

function checkpoint(
  directory: string,
  sessionID: string,
  summary: string,
  eventID?: string,
): void {
  summary = capCapture(summary)
  if (!summary.trim()) return
  const args = [
    "checkpoint",
    "--harness",
    "opencode",
    "--session",
    sessionID,
    "--cwd",
    directory,
    "--goal",
    "Complete the current OpenCode coding turn",
    "--summary-stdin",
  ]
  args.push(
    "--idempotency-key",
    automaticIdempotencyKey("opencode.assistant-checkpoint", [
      sessionID,
      eventID === undefined ? "missing" : "present",
      eventID ?? "",
      summary,
    ]),
  )
  run(args, summary)
}

export const SuperMem: Plugin = async ({ client, directory }) => ({
  "chat.message": async ({ sessionID }, out) => {
    const query = capCapture(textOf(out.parts as any[]))
    const memory = run([
      "recall",
      "--harness",
      "opencode",
      "--session",
      sessionID,
      "--cwd",
      directory,
      "--query-stdin",
      "--observe-prompt",
      "--event-id",
      String(out.message.id),
      "--format",
      "context",
    ], query)
    if (memory) {
      out.message.system = [out.message.system, memory].filter(Boolean).join("\n\n")
    }
  },

  "tool.execute.after": async (input, output) => {
    try {
      const toolName = String(input.tool ?? "")
      if (!toolName || isSuperMemTool(toolName)) return
      const sessionID = String(input.sessionID)
      const eventID = String(input.callID)
      const command = commandOf(input.args)
      const succeeded = toolSucceeded(output.metadata)
      const content = capCapture(
        toolEvidence(toolName, input.args, output, succeeded),
      )
      const args = [
        "observe",
        "--harness",
        "opencode",
        "--session",
        sessionID,
        "--cwd",
        directory,
        "--kind",
        toolEventKind(toolName),
        "--event-id",
        eventID,
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
      run(args, content)
    } catch {
      // Memory must never block OpenCode.
    }
  },

  event: async ({ event }) => {
    if (event.type !== "session.idle") return
    const sessionID = event.properties.sessionID
    try {
      const { data } = await client.session.messages({
        path: { id: sessionID },
        query: { directory },
      })
      const messages = (data ?? []) as any[]
      let last: any
      for (let index = messages.length - 1; index >= 0; index -= 1) {
        const candidate = messages[index]
        if (candidate?.info?.role !== "assistant") continue
        last = candidate
        break
      }
      if (!last) return
      const eventID = last.info?.id
      checkpoint(
        directory,
        sessionID,
        textOf(last.parts ?? []),
        eventID === undefined ? undefined : String(eventID),
      )
    } catch {
      // Memory must never block OpenCode.
    }
  },

  "experimental.session.compacting": async ({ sessionID }, out) => {
    const context = run([
      "recall",
      "--harness",
      "opencode",
      "--session",
      sessionID,
      "--cwd",
      directory,
      "--query-stdin",
      "--format",
      "context",
    ], "current project decisions constraints and unfinished work")
    if (context) out.context.push(context)
  },
})

export default SuperMem
