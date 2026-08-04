import { spawnSync } from "node:child_process"
import type { Plugin } from "@opencode-ai/plugin"

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

function textOf(parts: any[]): string {
  return parts
    .filter((part) => part?.type === "text" && !part.synthetic)
    .map((part) => String(part.text ?? ""))
    .filter(Boolean)
    .join("\n")
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
  if (eventID) args.push("--idempotency-key", `opencode:${sessionID}:${eventID}`)
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

  event: async ({ event }) => {
    if (event.type !== "session.idle") return
    const sessionID = event.properties.sessionID
    try {
      const { data } = await client.session.messages({
        path: { id: sessionID },
        query: { directory },
      })
      const messages = (data ?? []) as any[]
      const last = [...messages].reverse().find((item) => item?.info?.role === "assistant")
      if (!last) return
      checkpoint(directory, sessionID, textOf(last.parts ?? []), last.info?.id)
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
