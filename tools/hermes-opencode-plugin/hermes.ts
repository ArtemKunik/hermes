import { type Plugin, tool } from "@opencode-ai/plugin"

// ── Configuration ──────────────────────────────────────────────────────────

const CCTERM_PORT = process.env.CCTERM_PORT || process.env.VIBETUNNEL_RUST_PORT || "38080"
const BASE_URL = `http://localhost:${CCTERM_PORT}`
const AUTH_USER = process.env.CCTERM_USERNAME || process.env.VIBETUNNEL_RUST_USERNAME || "hp2"
const AUTH_PASS = process.env.CCTERM_PASSWORD || process.env.VIBETUNNEL_RUST_PASSWORD || ""
const BASIC_AUTH = AUTH_PASS ? `Basic ${Buffer.from(`${AUTH_USER}:${AUTH_PASS}`).toString("base64")}` : null
const TOOL_TIMEOUT_MS = 30_000

// ── HTTP Bridge ─────────────────────────────────────────────────────────────

interface CallToolResponse {
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
}

async function hermesCall(name: string, args: Record<string, unknown>) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), TOOL_TIMEOUT_MS)

  try {
    const headers: Record<string, string> = { "Content-Type": "application/json" }
    if (BASIC_AUTH) headers["Authorization"] = BASIC_AUTH

    const res = await fetch(`${BASE_URL}/api/hermes/call`, {
      method: "POST",
      headers,
      body: JSON.stringify({ name, arguments: args }),
      signal: controller.signal,
    })

    if (!res.ok) {
      const text = await res.text().catch(() => "no body")
      return { output: `Hermes HTTP ${res.status}: ${text}` }
    }

    const data: CallToolResponse = await res.json()

    if (!data.success) {
      return { output: `Hermes error: ${data.error || "unknown"}` }
    }

    const result = data.result
    const text = typeof result === "string" ? result : JSON.stringify(result, null, 2)
    return { output: text }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    if (msg.includes("abort")) return { output: "Hermes call timed out" }
    return { output: `Hermes unreachable — is ccterm running on :${CCTERM_PORT}?\n${msg}` }
  } finally {
    clearTimeout(timer)
  }
}

// ── Auto-index debounce ─────────────────────────────────────────────────────

let indexTimer: ReturnType<typeof setTimeout> | null = null

function scheduleIndex() {
  if (indexTimer) clearTimeout(indexTimer)
  indexTimer = setTimeout(() => {
    indexTimer = null
    hermesCall("hermes_index", {}).catch(() => {})
  }, 5_000)
}

// ── Plugin ──────────────────────────────────────────────────────────────────

export const HermesPlugin: Plugin = async () => {
  hermesCall("hermes_stats", {}).then(
    () => console.log("[hermes] connected to ccterm proxy"),
    () => console.warn("[hermes] ccterm not reachable — tools will show errors"),
  )
  .catch(() => {})

  return {
    tool: {
      hermes_search: tool({
        description: "Search the codebase knowledge graph. Returns compact pointers (file path, line range, summary) instead of full file content. Pass an optional 'goal' hint to bias results toward your specific information need.",
        args: {
          query: tool.schema.string().describe("Natural-language or keyword search query"),
          goal: tool.schema.string().optional().describe("Optional goal hint describing the agent's current information need"),
        },
        async execute(args) {
          return hermesCall("hermes_search", { query: args.query, goal: args.goal, top_k: 10 })
        },
      }),

      hermes_fetch: tool({
        description: "Fetch full file content for a knowledge-graph node by node ID (returned by hermes_search). Use this to read the actual code for a search hit.",
        args: {
          node_id: tool.schema.string().describe("Node ID from a previous hermes_search result"),
        },
        async execute(args) {
          return hermesCall("hermes_fetch", { node_id: args.node_id })
        },
      }),

      hermes_recall: tool({
        description: "Recall prior work on a topic. Searches Hermes session memory for related decisions, what was tried, what worked/failed. Use BEFORE starting implementation to avoid repeating past dead ends.",
        args: {
          query: tool.schema.string().describe("Topic or problem to recall prior work for"),
        },
        async execute(args) {
          return hermesCall("hermes_recall", { query: args.query })
        },
      }),

      hermes_remember: tool({
        description: "Save a summary of the current session to Hermes memory. Creates a structured markdown file that enables recall of past decisions in future sessions.",
        args: {
          topic: tool.schema.string().describe("Session topic or title"),
          summary: tool.schema.string().describe("Concise summary of the conversation and outcomes"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags (e.g. ['arch-decision', 'cosmos-db'])"),
          decisions: tool.schema.array(tool.schema.string()).optional().describe("Key decisions made with rationale"),
          actions: tool.schema.array(tool.schema.string()).optional().describe("Remaining action items"),
        },
        async execute(args) {
          return hermesCall("hermes_remember", args)
        },
      }),

      hermes_write_decision: tool({
        description: "Create a structured decision document. Use when a non-trivial problem is resolved — records context, what was tried, root cause, and tags that drive future hermes_recall accuracy.",
        args: {
          title: tool.schema.string().describe("Decision title"),
          status: tool.schema.enum(["OPEN", "PARTIALLY RESOLVED", "RESOLVED", "ABANDONED"]).optional().describe("Current status"),
          context: tool.schema.string().optional().describe("What problem, which component, why it matters"),
          what_worked: tool.schema.array(tool.schema.string()).optional().describe("Approaches that worked"),
          what_failed: tool.schema.array(tool.schema.string()).optional().describe("Dead-ends and why they failed"),
          root_cause: tool.schema.string().optional().describe("Best understanding of root cause"),
          next_steps: tool.schema.array(tool.schema.string()).optional().describe("Untried next actions"),
          related_files: tool.schema.array(tool.schema.string()).optional().describe("Relevant file paths"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("3-6 kebab-case tags: component, error-pattern, fix-type"),
        },
        async execute(args) {
          return hermesCall("hermes_write_decision", args)
        },
      }),

      hermes_fact: tool({
        description: "Record a persistent fact into Hermes temporal store. Facts are typed knowledge claims with optional TTL, tags, and confidence. Use to persist constraints, assumptions, and learnings across sessions.",
        args: {
          content: tool.schema.string().describe("The fact text"),
          fact_type: tool.schema.string().optional().describe("decision | constraint | assumption | observation | dependency | learning | architecture | api_contract | error_pattern"),
          topic: tool.schema.string().optional().describe("Free-text grouping label"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
          confidence: tool.schema.number().min(0).max(1).optional().describe("Confidence 0.0-1.0"),
          ttl: tool.schema.string().optional().describe("ISO 8601 duration like 'P7D' (7 days) or 'PT1H' (1 hour)"),
        },
        async execute(args) {
          return hermesCall("hermes_fact", args)
        },
      }),

      hermes_facts: tool({
        description: "List active facts from the temporal store. Optionally filter by type, topic, or tags. Use include_expired=true to include stale facts.",
        args: {
          fact_type: tool.schema.string().optional().describe("Filter by type"),
          topic: tool.schema.string().optional().describe("Filter by topic"),
          limit: tool.schema.number().optional().describe("Max results (default: 50)"),
          include_expired: tool.schema.boolean().optional().describe("Include expired facts"),
        },
        async execute(args) {
          return hermesCall("hermes_facts", args)
        },
      }),

      hermes_lint: tool({
        description: "Scan the knowledge graph for architecture violations: layer breaches, size limits, safety anti-patterns (unwrap/expect in prod), concurrency issues, and SQL injection risks. Default mode='summary' for compact output.",
        args: {
          mode: tool.schema.enum(["summary", "iterative", "full"]).optional().describe("summary (compact), iterative (drill one rule), full (all violations)"),
          rule_id: tool.schema.string().optional().describe("Drill into one rule (requires mode='iterative')"),
          severity_min: tool.schema.enum(["error", "warning", "info"]).optional().describe("Minimum severity (default: warning)"),
          rules: tool.schema.array(tool.schema.string()).optional().describe("Rule IDs to check, e.g. ['LAYER-001', 'SAFETY-001']"),
          scope: tool.schema.string().optional().describe("Limit to a path, directory, or crate"),
        },
        async execute(args) {
          return hermesCall("hermes_lint_architecture", args)
        },
      }),

      hermes_repo_map: tool({
        description: "Generate a compact repository map showing all code symbols (functions, structs, traits, enums) ranked by reference count. Gives a global architecture overview without reading entire files.",
        args: {
          max_tokens: tool.schema.number().optional().describe("Token budget (default: 2048)"),
        },
        async execute(args) {
          return hermesCall("hermes_repo_map", { max_tokens: args.max_tokens ?? 2048 })
        },
      }),

      hermes_constraints: tool({
        description: "Return architecture constraints for a specific file before generating code. Includes layer classification, applicable rules, naming conventions, and line budgets. Call BEFORE writing new code.",
        args: {
          file_path: tool.schema.string().describe("Target file path (relative or absolute)"),
          line_start: tool.schema.number().optional().describe("Start line"),
          line_end: tool.schema.number().optional().describe("End line"),
        },
        async execute(args) {
          return hermesCall("hermes_constraints", args)
        },
      }),

      hermes_review: tool({
        description: "Run an LLM-driven quality review of source files under a given path. Checks against 14 architectural dimensions (QD-01..QD-14). Returns findings with scores.",
        args: {
          path: tool.schema.string().optional().describe("Relative path to review (defaults to project root)"),
          dim: tool.schema.string().optional().describe("Single dimension filter (e.g. 'QD-01'). Omit for all 14 dimensions."),
          tier: tool.schema.enum(["T1", "T2", "T3", "T4"]).optional().describe("Minimum tier filter"),
        },
        async execute(args) {
          return hermesCall("hermes_quality_review", args)
        },
      }),

      hermes_index: tool({
        description: "Re-index project files into the Hermes knowledge graph. Run after adding or changing files to keep search results current.",
        args: {},
        async execute() {
          return hermesCall("hermes_index", {})
        },
      }),

      hermes_stats: tool({
        description: "Return token savings and usage statistics across all Hermes sessions. Shows pointer-economy savings vs traditional RAG.",
        args: {},
        async execute() {
          return hermesCall("hermes_stats", {})
        },
      }),
    },

    "tool.execute.after": async (input) => {
      if (["bash", "write", "edit"].includes(input.tool)) {
        scheduleIndex()
      }
    },

    "experimental.session.compacting": async (_input, output) => {
      try {
        const result = await hermesCall("hermes_recall", { query: "recent work decisions context" })
        if (!result.output.startsWith("Hermes unreachable") && !result.output.startsWith("Hermes error") && !result.output.startsWith("Hermes HTTP")) {
          output.context.push(
            "## Prior Session Context (from Hermes)\n" +
            "The following was recalled from Hermes session memory:\n" +
            result.output
          )
        }
      } catch {
        // graceful degradation — compaction proceeds without Hermes context
      }
    },
  }
}
