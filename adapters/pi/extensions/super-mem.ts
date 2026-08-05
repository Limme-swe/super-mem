import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

const TIMEOUT_MS = 1800
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
    run([
      "checkpoint",
      ...base(ctx),
      "--goal",
      "Complete the current Pi coding turn",
      "--summary-stdin",
      "--idempotency-key",
      automaticIdempotencyKey("pi.agent-checkpoint", [sessionID, summary]),
    ], summary)
    latestGoal = ""
    latestAssistant = ""
  }
}
