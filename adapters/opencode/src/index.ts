import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import type { Plugin } from "@opencode-ai/plugin"

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
