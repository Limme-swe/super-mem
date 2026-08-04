import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

const TIMEOUT_MS = 1800
const MAX_CAPTURE_BYTES = 64 * 1024
const CAPTURE_TRUNCATION_MARKER =
  "\n… [super-mem automatic capture truncated; middle omitted] …\n"

function capCapture(value: string): string {
  if (Buffer.byteLength(value, "utf8") <= MAX_CAPTURE_BYTES) return value
  const symbols = Array.from(value)
  const contentBudget =
    MAX_CAPTURE_BYTES - Buffer.byteLength(CAPTURE_TRUNCATION_MARKER, "utf8")
  const headBudget = Math.floor(contentBudget / 2)
  const tailBudget = contentBudget - headBudget
  let head = ""
  let used = 0
  for (const symbol of symbols) {
    const size = Buffer.byteLength(symbol, "utf8")
    if (used + size > headBudget) break
    head += symbol
    used += size
  }
  const tail: string[] = []
  used = 0
  for (let index = symbols.length - 1; index >= 0; index -= 1) {
    const symbol = symbols[index]
    if (symbol === undefined) continue
    const size = Buffer.byteLength(symbol, "utf8")
    if (used + size > tailBudget) break
    tail.push(symbol)
    used += size
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
  return message.content
    .filter((part: any) => part?.type === "text")
    .map((part: any) => String(part.text ?? ""))
    .filter(Boolean)
    .join("\n")
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
    run([
      "checkpoint",
      ...base(ctx),
      "--summary-stdin",
      "--idempotency-key",
      `pi:${ctx.sessionManager.getSessionId()}:compact:${String(entry.id)}`,
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
    const digest = createHash("sha256").update(summary).digest("hex").slice(0, 24)
    run([
      "checkpoint",
      ...base(ctx),
      "--goal",
      "Complete the current Pi coding turn",
      "--summary-stdin",
      "--idempotency-key",
      `pi:${ctx.sessionManager.getSessionId()}:${digest}`,
    ], summary)
    latestGoal = ""
    latestAssistant = ""
  }
}
