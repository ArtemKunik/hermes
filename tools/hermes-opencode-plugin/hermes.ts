import { type Plugin, tool } from "@opencode-ai/plugin"

// ── Configuration ──────────────────────────────────────────────────────────

const CCTERM_PORT = process.env.CCTERM_PORT || process.env.VIBETUNNEL_RUST_PORT || "38080"
const BASE_URL = `http://localhost:${CCTERM_PORT}`
const AUTH_USER = process.env.CCTERM_USERNAME || process.env.VIBETUNNEL_RUST_USERNAME || "hp2"
const AUTH_PASS = process.env.CCTERM_PASSWORD || process.env.VIBETUNNEL_RUST_PASSWORD || ""
const BASIC_AUTH = AUTH_PASS ? `Basic ${Buffer.from(`${AUTH_USER}:${AUTH_PASS}`).toString("base64")}` : null
const TOOL_TIMEOUT_MS = 30_000
const AGENTS_MD_INJECT = process.env.HERMES_AUTO_INJECT_SYMBOLS !== "false"
const HERMES_MISSION_ID = process.env.HERMES_MISSION_ID || null
const SESSION_ID = process.env.OPENCODE_SESSION_ID || `session-${Date.now()}`
const AGENT_ID = process.env.OPENCODE_AGENT_ID || process.env.USER || "agent"

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
let heartbeatTimer: ReturnType<typeof setTimeout> | null = null

function scheduleIndex() {
  if (indexTimer) clearTimeout(indexTimer)
  indexTimer = setTimeout(() => {
    indexTimer = null
    hermesCall("hermes_index", {}).catch(() => {})
  }, 5_000)
}

// ── Mission heartbeat ───────────────────────────────────────────────────────

function scheduleHeartbeat() {
  if (!HERMES_MISSION_ID || heartbeatTimer) return
  heartbeatTimer = setTimeout(() => {
    heartbeatTimer = null
    hermesCall("hermes_mission_heartbeat", { mission_id: HERMES_MISSION_ID }).catch(() => {})
    scheduleHeartbeat()
  }, 60_000)
}

// ── Helper: wrap a tool call ────────────────────────────────────────────────

function bridge(name: string, args: Record<string, unknown>) {
  return hermesCall(name, args)
}

function toolDef<Args extends Parameters<typeof tool>[0]["args"]>(p: {
  description: string
  args: Args
  execute: (args: any) => ReturnType<typeof hermesCall>
}) {
  return tool({
    description: p.description,
    args: p.args,
    async execute(args: any) { return p.execute(args) },
  })
}

// ── Plugin ──────────────────────────────────────────────────────────────────

export const HermesPlugin: Plugin = async () => {
  hermesCall("hermes_stats", {}).then(
    () => console.log("[hermes] connected to ccterm proxy"),
    () => console.warn("[hermes] ccterm not reachable — tools will show errors"),
  ).catch(() => {})

  // Auto-inject symbols into AGENTS.md on session start.
  if (AGENTS_MD_INJECT) {
    hermesCall("hermes_inject_symbols", { budget: 2000 }).catch(() => {})
  }

  // Mission session binding.
  if (HERMES_MISSION_ID) {
    console.log(`[hermes] session bound to mission ${HERMES_MISSION_ID}`)
    hermesCall("hermes_mission_event", {
      mission_id: HERMES_MISSION_ID,
      event_type: "session_bind",
      data: { session_id: SESSION_ID, agent_id: AGENT_ID, bound_at: new Date().toISOString() },
    }).catch(() => {})
    scheduleHeartbeat()
  }

  return {
    tool: {
      // ── Core Tools ──────────────────────────────────────────────────────────
      hermes_search: toolDef({
        description: "Search the codebase knowledge graph. Returns compact pointers (file, line range, summary) instead of full content. Optionally pass a 'goal' hint to bias results toward a specific information need.",
        args: { query: tool.schema.string().describe("Natural-language or keyword search query"), goal: tool.schema.string().optional().describe("Optional goal hint for search bias") },
        execute: (args) => bridge("hermes_search", { query: args.query, goal: args.goal, top_k: 10 }),
      }),

      hermes_fetch: toolDef({
        description: "Fetch full file content for a knowledge-graph node by node ID (returned by hermes_search).",
        args: { node_id: tool.schema.string().describe("Node ID from a previous hermes_search result") },
        execute: (args) => bridge("hermes_fetch", { node_id: args.node_id }),
      }),

      hermes_lookup: toolDef({
        description: "O(1) symbol lookup: find where a function, struct, class, or type is defined. Returns file path, line, kind, exported flag, and methods.",
        args: { symbol_name: tool.schema.string().describe("Symbol name to find (e.g. 'verify_token', 'AuthService')") },
        execute: (args) => bridge("hermes_lookup", { symbol_name: args.symbol_name }),
      }),

      hermes_file_symbols: toolDef({
        description: "List all symbols defined in a file with line numbers, kinds, and exported status.",
        args: { file_path: tool.schema.string().describe("File path relative to project root (e.g. 'src/auth.rs')") },
        execute: (args) => bridge("hermes_file_symbols", { file_path: args.file_path }),
      }),

      hermes_index: toolDef({
        description: "Re-index project files into the knowledge graph. Run after adding or changing files.",
        args: {},
        execute: () => bridge("hermes_index", {}),
      }),

      hermes_backfill: toolDef({
        description: "Backfill stored content token counts for existing nodes (retro-fill).",
        args: {},
        execute: () => bridge("hermes_backfill", {}),
      }),

      hermes_stats: toolDef({
        description: "Return cumulative token savings statistics across all Hermes sessions.",
        args: {},
        execute: () => bridge("hermes_stats", {}),
      }),

      hermes_mcp_status: toolDef({
        description: "Return structured MCP runtime health: node counts, index state, DB config, search cache stats, tool inventory.",
        args: {},
        execute: () => bridge("hermes_mcp_status", {}),
      }),

      hermes_tools: toolDef({
        description: "Return a routed subset of tool schemas for a given intent. Use before selecting a Hermes tool to reduce schema overload.",
        args: { intent: tool.schema.string().describe("Intent string like 'recall previous work' or 'quality review'") },
        execute: (args) => bridge("hermes_tools", { intent: args.intent }),
      }),

      // ── Memory / Recall ──────────────────────────────────────────────────────

      hermes_recall: toolDef({
        description: "Recall prior work on a topic. Returns structured context: what was tried, what worked/failed, recommended next steps. Use BEFORE starting implementation.",
        args: { query: tool.schema.string().describe("Topic or problem to recall"), topic: tool.schema.string().optional().describe("Alias for query (compat)") },
        execute: (args) => bridge("hermes_recall", { query: args.query || args.topic }),
      }),

      hermes_remember: toolDef({
        description: "Save a session summary to memory. Creates a structured markdown in memory/sessions/ for future recall.",
        args: {
          topic: tool.schema.string().describe("Session topic / title"),
          summary: tool.schema.string().describe("Concise conversation summary"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
          files_touched: tool.schema.array(tool.schema.string()).optional().describe("Files modified this session"),
          decisions: tool.schema.array(tool.schema.string()).optional().describe("Key decisions"),
          problems: tool.schema.array(tool.schema.string()).optional().describe("Problems encountered"),
          actions: tool.schema.array(tool.schema.string()).optional().describe("Remaining action items"),
        },
        execute: (args) => bridge("hermes_remember", args),
      }),

      hermes_write_decision: toolDef({
        description: "Create a structured decision document in memory/decisions/. Use when a non-trivial problem is RESOLVED.",
        args: {
          title: tool.schema.string().describe("Decision title"),
          status: tool.schema.enum(["OPEN", "PARTIALLY RESOLVED", "RESOLVED", "ABANDONED"]).optional().describe("Current status"),
          context: tool.schema.string().optional().describe("Problem, component, why it matters"),
          what_worked: tool.schema.array(tool.schema.string()).optional().describe("Approaches that worked"),
          what_failed: tool.schema.array(tool.schema.string()).optional().describe("Dead-end approaches"),
          root_cause: tool.schema.string().optional().describe("Best understanding of root cause"),
          next_steps: tool.schema.array(tool.schema.string()).optional().describe("Untried next actions"),
          related_files: tool.schema.array(tool.schema.string()).optional().describe("Relevant file paths"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("3-6 kebab-case tags"),
        },
        execute: (args) => bridge("hermes_write_decision", args),
      }),

      hermes_fact: toolDef({
        description: "Record a persistent fact into the temporal store. Facts are typed knowledge claims with optional TTL and confidence.",
        args: {
          content: tool.schema.string().describe("The fact text"),
          fact_type: tool.schema.string().optional().describe("decision | constraint | assumption | observation | dependency | learning | architecture | api_contract | error_pattern"),
          topic: tool.schema.string().optional().describe("Free-text grouping label"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
          confidence: tool.schema.number().min(0).max(1).optional().describe("Confidence 0.0-1.0"),
          ttl: tool.schema.string().optional().describe("ISO 8601 duration like 'P7D' or 'PT1H'"),
        },
        execute: (args) => bridge("hermes_fact", args),
      }),

      hermes_facts: toolDef({
        description: "List active facts from the temporal store. Optionally filter by type, topic, or tags.",
        args: {
          fact_type: tool.schema.string().optional().describe("Filter by type"),
          topic: tool.schema.string().optional().describe("Filter by topic"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Filter to facts containing ALL tags"),
          limit: tool.schema.number().optional().describe("Max results (default: 50)"),
          include_expired: tool.schema.boolean().optional().describe("Include expired facts"),
        },
        execute: (args) => bridge("hermes_facts", args),
      }),

      hermes_fact_expire: toolDef({
        description: "Expire (soft-delete) an active fact, optionally linking the replacement.",
        args: {
          fact_id: tool.schema.string().describe("ID of the fact to expire"),
          superseded_by: tool.schema.string().optional().describe("Optional replacement fact ID"),
        },
        execute: (args) => bridge("hermes_fact_expire", args),
      }),

      hermes_memory_stats: toolDef({
        description: "Return memory usage statistics: sessions saved, recall hits, memory hit rate.",
        args: {},
        execute: () => bridge("hermes_memory_stats", {}),
      }),

      hermes_battery_check: toolDef({
        description: "Determine if a periodic battery-change review is due (defaults 10 sessions / 7 days).",
        args: {
          session_threshold: tool.schema.number().optional().describe("Session count threshold"),
          day_threshold: tool.schema.number().optional().describe("Days since last review threshold"),
        },
        execute: (args) => bridge("hermes_battery_check", args),
      }),

      hermes_query_memory: toolDef({
        description: "Search Qdrant semantic_memory for chunks relevant to a query. Use to ground responses in past project knowledge.",
        args: { query: tool.schema.string().describe("Natural-language query"), limit: tool.schema.number().optional().describe("Max chunks (default: 5)") },
        execute: (args) => bridge("hermes_query_memory", { query: args.query, limit: args.limit ?? 5 }),
      }),

      hermes_get_core_facts: toolDef({
        description: "Read memory/CORE_FACTS.md — the durable project knowledge base with service topology, infrastructure, conventions.",
        args: {},
        execute: () => bridge("hermes_get_core_facts", {}),
      }),

      hermes_compact_session: toolDef({
        description: "Compact active session context into a continuation-focused handover artifact. Optionally persists to memory/handover/.",
        args: {
          topic: tool.schema.string().optional().describe("Primary task/topic for the handover"),
          summary: tool.schema.string().optional().describe("Current status before compaction"),
          recent_messages: tool.schema.array(tool.schema.string()).optional().describe("Recent steps to preserve"),
          accomplished: tool.schema.array(tool.schema.string()).optional().describe("Completed progress items"),
          files_touched: tool.schema.array(tool.schema.string()).optional().describe("Files for continuation"),
          decisions: tool.schema.array(tool.schema.string()).optional().describe("Important decisions"),
          problems: tool.schema.array(tool.schema.string()).optional().describe("Current blockers"),
          actions: tool.schema.array(tool.schema.string()).optional().describe("Next actions"),
          active_constraints: tool.schema.array(tool.schema.string()).optional().describe("Architectural constraints"),
          continuation_prompt: tool.schema.string().optional().describe("Ready-to-run continuation prompt"),
          target_token_budget: tool.schema.number().optional().describe("Token budget for summary (default: 1200)"),
          persist_handover: tool.schema.boolean().optional().describe("Write handover to memory/handover/ and index it"),
        },
        execute: (args) => bridge("hermes_compact_session", args),
      }),

      // ── Architecture / Lint ──────────────────────────────────────────────────

      hermes_lint: toolDef({
        description: "Scan for architecture violations: layer breaches, size limits, safety anti-patterns, concurrency, SQL injection. Default mode='summary' for compact output. Use mode='iterative' with rule_id to drill into a specific rule.",
        args: {
          mode: tool.schema.enum(["summary", "iterative", "full"]).optional().describe("Output mode (default: summary)"),
          rule_id: tool.schema.string().optional().describe("Drill into one rule (requires mode='iterative')"),
          severity_min: tool.schema.enum(["error", "warning", "info"]).optional().describe("Minimum severity (default: warning)"),
          scope: tool.schema.string().optional().describe("Limit to a path, directory, or crate"),
          rules: tool.schema.array(tool.schema.string()).optional().describe("Specific rule IDs (e.g. ['LAYER-001','SIZE-001'])"),
          auto_scope: tool.schema.boolean().optional().describe("Auto-restrict to git-changed files (default: true)"),
        },
        execute: (args) => bridge("hermes_lint_architecture", args),
      }),

      hermes_heal_violations: toolDef({
        description: "Generate constrained healing candidates from lint output. Phase 1 auto-selects SAFETY-001/002/003 with gpt-5-mini.",
        args: {
          scope: tool.schema.string().optional().describe("Path/crate scope"),
          severity_min: tool.schema.enum(["error", "warning", "info"]).optional().describe("Severity floor"),
          rules: tool.schema.array(tool.schema.string()).optional().describe("Rule filter"),
          dry_run: tool.schema.boolean().optional().describe("Return candidates without applying (default: true)"),
          max_items: tool.schema.number().optional().describe("Max candidates (default: 25)"),
        },
        execute: (args) => bridge("hermes_heal_violations", args),
      }),

      hermes_constraints: toolDef({
        description: "Return architecture constraints for a specific file before generating code: layer, rules, naming convention, size budget. Call BEFORE writing new code.",
        args: {
          file_path: tool.schema.string().describe("Target file path"),
          line_start: tool.schema.number().optional().describe("Start line"),
          line_end: tool.schema.number().optional().describe("End line"),
        },
        execute: (args) => bridge("hermes_constraints", args),
      }),

      hermes_impact_analysis: toolDef({
        description: "Trace the knowledge graph to find all downstream dependents of a symbol. Returns affected files plus pre-computed blast-radius score.",
        args: { symbol_name: tool.schema.string().describe("Symbol name to analyze (e.g. 'User', 'fetch_data')") },
        execute: (args) => bridge("hermes_impact_analysis", { symbol_name: args.symbol_name }),
      }),

      hermes_blast_score: toolDef({
        description: "Look up the pre-computed blast-radius score for a symbol or file. Returns direct/transitive counts, score, risk level (HIGH/MEDIUM/LOW).",
        args: { query: tool.schema.string().describe("Symbol name or file path to query") },
        execute: (args) => bridge("hermes_blast_score", { query: args.query }),
      }),

      hermes_high_blast: toolDef({
        description: "Return top-N files/symbols ranked by blast score above a threshold. Use to identify highest-impact areas before refactoring.",
        args: {
          threshold: tool.schema.number().optional().describe("Min blast score (default: 1.0)"),
          limit: tool.schema.number().optional().describe("Max results (default: 20)"),
        },
        execute: (args) => bridge("hermes_high_blast", { threshold: args.threshold ?? 1.0, limit: args.limit ?? 20 }),
      }),

      hermes_repo_map: toolDef({
        description: "Generate a token-budgeted repository map of all code symbols (functions, structs, traits, enums) ranked by reference count.",
        args: { max_tokens: tool.schema.number().optional().describe("Token budget (default: 2048)") },
        execute: (args) => bridge("hermes_repo_map", { max_tokens: args.max_tokens ?? 2048 }),
      }),

      hermes_test_coverage_map: toolDef({
        description: "Map test→implementation edges. Returns covered symbols, uncovered symbols, and overall coverage ratio.",
        args: {
          symbol: tool.schema.string().optional().describe("Specific symbol to inspect"),
          scope: tool.schema.string().optional().describe("File/directory scope filter"),
        },
        execute: (args) => bridge("hermes_test_coverage_map", args),
      }),

      hermes_search_misses: toolDef({
        description: "Post-mortem report of zero-result searches. Use to find indexing gaps.",
        args: {
          since_days: tool.schema.number().optional().describe("Restrict to last N days"),
          top_k: tool.schema.number().optional().describe("Top repeated misses (default: 10)"),
        },
        execute: (args) => bridge("hermes_search_misses", { since_days: args.since_days, top_k: args.top_k ?? 10 }),
      }),

      hermes_validate_env: toolDef({
        description: "Check environment variable name against the config registry. Returns known/unknown with similar name suggestions.",
        args: { env_var: tool.schema.string().describe("Environment variable name to validate") },
        execute: (args) => bridge("hermes_validate_env", { env_var: args.env_var }),
      }),

      hermes_validate_symbols: toolDef({
        description: "Validate symbol names against the knowledge graph. Returns exists/not-found with closest-known suggestions.",
        args: { symbols: tool.schema.array(tool.schema.string()).describe("Symbol names to validate (e.g. ['ingest_file', 'XrefExtractor'])") },
        execute: (args) => bridge("hermes_validate_symbols", { symbols: args.symbols }),
      }),

      hermes_check_consistency: toolDef({
        description: "Scan for environment variable inconsistencies: used but not defined, defined but never used.",
        args: {},
        execute: () => bridge("hermes_check_consistency", {}),
      }),

      hermes_scan_duplicates: toolDef({
        description: "Scan a function/struct signature for semantically similar symbols using vector embeddings.",
        args: { signature: tool.schema.string().describe("Function/struct signature or preview text") },
        execute: (args) => bridge("hermes_scan_duplicates", { signature: args.signature }),
      }),

      hermes_prepare_commit_message: toolDef({
        description: "Generate a commit message body with structured trailers (Task-Model, Decision-Doc, etc.) for traceable context.",
        args: {
          subject: tool.schema.string().describe("Conventional commit subject"),
          body: tool.schema.string().optional().describe("Optional commit body"),
          task_model: tool.schema.string().optional().describe("Task model URI"),
          decision_doc: tool.schema.string().optional().describe("Path to decision doc"),
          session_note: tool.schema.string().optional().describe("Path to session note"),
          docs: tool.schema.array(tool.schema.string()).optional().describe("Doc paths"),
          changes: tool.schema.array(tool.schema.string()).optional().describe("Changed file paths"),
        },
        execute: (args) => bridge("hermes_prepare_commit_message", args),
      }),

      // ── Quality Review ───────────────────────────────────────────────────────

      hermes_review: toolDef({
        description: "Run LLM quality review of files against 14 architectural dimensions (QD-01..QD-14). Returns findings with scores.",
        args: {
          path: tool.schema.string().optional().describe("Relative path to review (defaults to project root)"),
          dim: tool.schema.string().optional().describe("Single dimension filter (e.g. 'QD-01')"),
          tier: tool.schema.enum(["T1", "T2", "T3", "T4"]).optional().describe("Minimum tier filter"),
        },
        execute: (args) => bridge("hermes_quality_review", args),
      }),

      hermes_quality_score: toolDef({
        description: "Return quality scores per module and project overall (0-100). Optionally include trend vs previous scan.",
        args: {
          module: tool.schema.string().optional().describe("Filter to a crate/module"),
          trend: tool.schema.boolean().optional().describe("Include score delta vs previous scan"),
        },
        execute: (args) => bridge("hermes_quality_score", args),
      }),

      hermes_quality_next: toolDef({
        description: "Return the single highest-priority open finding across the project or a specific module.",
        args: { module: tool.schema.string().optional().describe("Restrict to a module") },
        execute: (args) => bridge("hermes_quality_next", args),
      }),

      hermes_quality_resolve: toolDef({
        description: "Mark a quality finding as resolved. Penalty is removed and scores recomputed.",
        args: { id: tool.schema.string().describe("Finding ID (e.g. 'Q-A1B2C3D4')") },
        execute: (args) => bridge("hermes_quality_resolve", { id: args.id }),
      }),

      hermes_quality_wontfix: toolDef({
        description: "Mark a finding as won't-fix with mandatory reason. Penalty is halved (not removed).",
        args: { id: tool.schema.string().describe("Finding ID"), reason: tool.schema.string().describe("Why this is acceptable") },
        execute: (args) => bridge("hermes_quality_wontfix", { id: args.id, reason: args.reason }),
      }),

      hermes_quality_baseline: toolDef({
        description: "Snapshot current arch-lint violations as drift baseline for hermes_quality_drift comparison.",
        args: {},
        execute: () => bridge("hermes_quality_baseline", {}),
      }),

      hermes_quality_drift: toolDef({
        description: "Compare current violations against baseline. Returns regressions, improvements, per-rule deltas, trend.",
        args: {},
        execute: () => bridge("hermes_quality_drift", {}),
      }),

      hermes_quality_dismiss: toolDef({
        description: "Dismiss a quality finding. Disappears from active lists but stays in DB.",
        args: { id: tool.schema.string().describe("Finding ID"), reason: tool.schema.string().optional().describe("Dismissal reason") },
        execute: (args) => bridge("hermes_quality_dismiss", args),
      }),

      hermes_lint_dismiss: toolDef({
        description: "Dismiss a lint violation or skill candidate by ID.",
        args: {
          item_type: tool.schema.enum(["violation", "skill_candidate"]).describe("Type of item to dismiss"),
          item_id: tool.schema.string().describe("Unique identifier (fingerprint or name)"),
          reason: tool.schema.string().optional().describe("Dismissal reason"),
        },
        execute: (args) => bridge("hermes_lint_dismiss", args),
      }),

      hermes_dismissed_list: toolDef({
        description: "List all dismissed items (findings, violations, skill candidates).",
        args: {
          item_type: tool.schema.enum(["finding", "violation", "skill_candidate"]).optional().describe("Filter by type"),
          limit: tool.schema.number().optional().describe("Max results (default: 100)"),
        },
        execute: (args) => bridge("hermes_dismissed_list", args),
      }),

      hermes_auto_dismiss: toolDef({
        description: "Auto-dismiss open quality findings older than N days (default 30).",
        args: { max_age_days: tool.schema.number().optional().describe("Max age in days before dismissal (default: 30)") },
        execute: (args) => bridge("hermes_auto_dismiss", { max_age_days: args.max_age_days ?? 30 }),
      }),

      // ── Missions ─────────────────────────────────────────────────────────────

      hermes_mission_start: toolDef({
        description: "Create a new mission in 'preflight' status. A mission is a stateful container for multi-step agent work.",
        args: {
          title: tool.schema.string().describe("Short mission title"),
          description: tool.schema.string().optional().describe("Detailed description"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
          checklist: tool.schema.array(tool.schema.string()).optional().describe("Deliverable tasks"),
          repo_id: tool.schema.string().optional().describe("Repository scope"),
        },
        execute: (args) => bridge("hermes_mission_start", args),
      }),

      hermes_mission_update: toolDef({
        description: "Transition mission status and/or update metadata. Enforces state machine: preflight→active→landing→completed.",
        args: {
          mission_id: tool.schema.string().describe("Mission ID"),
          status: tool.schema.enum(["preflight", "active", "landing", "completed", "aborted"]).optional().describe("Target status"),
          title: tool.schema.string().optional().describe("Updated title"),
          description: tool.schema.string().optional().describe("Updated description"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Replaced tags"),
          checklist: tool.schema.array(tool.schema.string()).optional().describe("Replaced checklist"),
        },
        execute: (args) => bridge("hermes_mission_update", args),
      }),

      hermes_mission_event: toolDef({
        description: "Append a timestamped event to the mission log (phase transitions, artifacts, decisions, blockers).",
        args: {
          mission_id: tool.schema.string().describe("Mission ID"),
          event_type: tool.schema.string().describe("Event type: phase_enter | artifact | decision | task_progress | choice | blocked"),
          data: tool.schema.unknown().optional().describe("Event payload (e.g. {phase:'execution'})"),
        },
        execute: (args) => bridge("hermes_mission_event", args),
      }),

      hermes_mission_status: toolDef({
        description: "Retrieve current state of a mission including its full event log.",
        args: { mission_id: tool.schema.string().describe("Mission ID") },
        execute: (args) => bridge("hermes_mission_status", { mission_id: args.mission_id }),
      }),

      hermes_mission_list: toolDef({
        description: "List missions with optional filters (status, repo, limit).",
        args: {
          status: tool.schema.string().optional().describe("Filter by status"),
          repo_id: tool.schema.string().optional().describe("Filter by repository"),
          limit: tool.schema.number().optional().describe("Max records (default: 20)"),
        },
        execute: (args) => bridge("hermes_mission_list", args),
      }),

      hermes_mission_heartbeat: toolDef({
        description: "Send a liveness heartbeat for the active mission. Records session activity to prevent stale detection.",
        args: { mission_id: tool.schema.string().describe("Mission ID") },
        execute: (args) => bridge("hermes_mission_event", {
          mission_id: args.mission_id,
          event_type: "heartbeat",
          data: { session_id: SESSION_ID, ts: new Date().toISOString() },
        }),
      }),

      // ── Incidents / Knowledge Base ───────────────────────────────────────────

      hermes_log_incident: toolDef({
        description: "Open a new incident for a sub-product. Creates structured incident file under memory/incidents/.",
        args: {
          sub_product: tool.schema.string().describe("Affected sub-product (e.g. 'backend', 'telegram-gateway')"),
          title: tool.schema.string().describe("Short incident title"),
          severity: tool.schema.enum(["P0", "P1", "P2", "P3"]).optional().describe("Severity: P0=critical, P1=major, P2=partial, P3=minor"),
          symptoms: tool.schema.string().optional().describe("Observable symptoms and errors"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
        },
        execute: (args) => bridge("hermes_log_incident", args),
      }),

      hermes_resolve_incident: toolDef({
        description: "Resolve an open incident. Updates to RESOLVED and auto-writes KB article unless write_kb=false.",
        args: {
          sub_product: tool.schema.string().describe("Sub-product"),
          slug: tool.schema.string().describe("Incident slug from hermes_log_incident"),
          root_cause: tool.schema.string().optional().describe("Root cause explanation"),
          fix_summary: tool.schema.string().optional().describe("What was done"),
          files_changed: tool.schema.array(tool.schema.string()).optional().describe("Modified files"),
          lessons: tool.schema.string().optional().describe("Lessons learned"),
          write_kb: tool.schema.boolean().optional().describe("Auto-write KB article (default: true)"),
        },
        execute: (args) => bridge("hermes_resolve_incident", args),
      }),

      hermes_query_incidents: toolDef({
        description: "List incidents from the ledger with optional filters.",
        args: {
          sub_product: tool.schema.string().optional().describe("Filter by sub-product"),
          status: tool.schema.string().optional().describe("Filter by status: OPEN or RESOLVED"),
          severity: tool.schema.string().optional().describe("Filter by severity: P0-P3"),
        },
        execute: (args) => bridge("hermes_query_incidents", args),
      }),

      hermes_write_kb_article: toolDef({
        description: "Write a Knowledge Base article to memory/kb/<sub_product>/. Use for recurring issues and architectural gotchas.",
        args: {
          sub_product: tool.schema.string().optional().describe("Sub-product scope"),
          title: tool.schema.string().describe("Article title"),
          problem: tool.schema.string().optional().describe("Problem/symptoms"),
          root_cause: tool.schema.string().optional().describe("Root cause"),
          solution: tool.schema.string().optional().describe("Fix / solution steps"),
          prevention: tool.schema.string().optional().describe("Prevention advice"),
          related_incidents: tool.schema.array(tool.schema.string()).optional().describe("Related incident slugs"),
          tags: tool.schema.array(tool.schema.string()).optional().describe("Classification tags"),
        },
        execute: (args) => bridge("hermes_write_kb_article", args),
      }),

      hermes_search_kb: toolDef({
        description: "Search Knowledge Base articles in memory/kb/. Returns ranked matches.",
        args: {
          query: tool.schema.string().describe("Search query"),
          sub_product: tool.schema.string().optional().describe("Scope to sub-product"),
        },
        execute: (args) => bridge("hermes_search_kb", args),
      }),

      // ── Skills ───────────────────────────────────────────────────────────────

      hermes_match_skills: toolDef({
        description: "Search the indexed skill library for reusable workflows matching a task query.",
        args: {
          query: tool.schema.string().describe("Natural-language task query"),
          scope: tool.schema.string().optional().describe("Scope: 'project', 'shared', or omit for all"),
        },
        execute: (args) => bridge("hermes_match_skills", args),
      }),

      hermes_fetch_skill: toolDef({
        description: "Fetch full content of a skill by file path or skill ID. Returns complete SKILL.md instructions.",
        args: { skill_path: tool.schema.string().describe("File path or skill ID") },
        execute: (args) => bridge("hermes_fetch_skill", { skill_path: args.skill_path }),
      }),

      // ── Tracks ───────────────────────────────────────────────────────────────

      hermes_list_tracks: toolDef({
        description: "List conductor tracks with status, progress, and next-step hints.",
        args: { status: tool.schema.string().optional().describe("Filter: unfinished, active, in-progress, planned, speccing, blocked, completed, all") },
        execute: (args) => bridge("hermes_list_tracks", { status: args.status }),
      }),

      hermes_resume_track: toolDef({
        description: "Prepare a continuation brief for an unfinished conductor track without changing code.",
        args: {
          track_id: tool.schema.string().optional().describe("Track ID like TRACK-062"),
          auto: tool.schema.boolean().optional().describe("Auto-pick best unfinished track"),
          status: tool.schema.string().optional().describe("When auto=true, limit to status bucket"),
        },
        execute: (args) => bridge("hermes_resume_track", args),
      }),

      // ── Slow Loop ────────────────────────────────────────────────────────────

      hermes_slow_loop_status: toolDef({
        description: "Return current status of the Hermes Slow Loop (digests, compaction, skill candidates).",
        args: {},
        execute: () => bridge("hermes_slow_loop_status", {}),
      }),

      hermes_generate_digest: toolDef({
        description: "Manually trigger daily digest generation for a specific date (YYYY-MM-DD).",
        args: { date: tool.schema.string().describe("Target date in YYYY-MM-DD format") },
        execute: (args) => bridge("hermes_generate_digest", { date: args.date }),
      }),

      hermes_compact_sessions: toolDef({
        description: "Manually trigger session compaction (archive sessions older than 14 days).",
        args: {},
        execute: () => bridge("hermes_compact_sessions", {}),
      }),

      hermes_generate_weekly_brief: toolDef({
        description: "Manually trigger weekly pattern detection and skill candidate generation.",
        args: {},
        execute: () => bridge("hermes_generate_weekly_brief", {}),
      }),

      hermes_approve_skill_candidate: toolDef({
        description: "Approve a candidate skill and promote it to the formal skill library.",
        args: { name: tool.schema.string().describe("Candidate skill name") },
        execute: (args) => bridge("hermes_approve_skill_candidate", { name: args.name }),
      }),

      hermes_reject_skill_candidate: toolDef({
        description: "Reject a candidate skill and archive it.",
        args: { name: tool.schema.string().describe("Candidate skill name") },
        execute: (args) => bridge("hermes_reject_skill_candidate", { name: args.name }),
      }),

      hermes_apply_proposal: toolDef({
        description: "Apply a drift correction proposal to the codebase.",
        args: { filename: tool.schema.string().describe("Proposal filename from memory/slow_loop/proposals/") },
        execute: (args) => bridge("hermes_apply_proposal", { filename: args.filename }),
      }),

      // ── Viz Graph Data ───────────────────────────────────────────────────────

      hermes_viz_graph: toolDef({
        description: "Return dependency graph data: nodes (with blast scores, risk, LOC) and edges (calls, imports). Compatible with d3 force layout.",
        args: {},
        execute: () => bridge("hermes_viz_graph", {}),
      }),
    },

    // ── Lifecycle Hooks ────────────────────────────────────────────────────────

    "tool.execute.after": async (input, output) => {
      // ── Mission auto-reporting ─────────────────────────────────────
      if (HERMES_MISSION_ID) {
        scheduleHeartbeat()

        const reportEvent = (eventType: string, data: Record<string, unknown>) => {
          hermesCall("hermes_mission_event", {
            mission_id: HERMES_MISSION_ID,
            event_type: eventType,
            data: { ...data, session_id: SESSION_ID },
          }).catch(() => {})
        }

        if (input.tool === "write" || input.tool === "edit") {
          const file = (input.args as any)?.filePath || (input.args as any)?.file_path || (input.args as any)?.path || "unknown"
          reportEvent("artifact", { file, tool: input.tool, action: "modified" })
        }

        if (input.tool === "hermes_write_decision") {
          reportEvent("decision", { title: (input.args as any)?.title || "untitled" })
        }

        if (input.tool === "bash") {
          const resultText = output?.output || ""
          if (resultText.includes("error") || resultText.includes("Error") || resultText.includes("FAILED")) {
            reportEvent("blocked", { error: resultText.substring(0, 500), tool: "bash" })
          }
        }
      }

      const modifiedTools = ["bash", "write", "edit"]

      if (modifiedTools.includes(input.tool)) {
        // Schedule auto-index
        scheduleIndex()

        // Blast-score write guard: check modified files for HIGH risk.
        // We try to extract file paths from tool output or args.
        const resultText = output?.output || ""
        const fileArg = (input.args as any)?.filePath || (input.args as any)?.file_path || (input.args as any)?.path
        const filesToCheck: string[] = []

        if (fileArg) {
          filesToCheck.push(fileArg as string)
        }

        // Parse tool output for file paths (bash tool often returns paths in output)
        if (resultText) {
          const pathMatches = resultText.matchAll(/(?:^|\s)([\w\-./]+\.(?:rs|ts|tsx|js|jsx|py))\b/g)
          for (const match of pathMatches) {
            if (match[1] && !filesToCheck.includes(match[1])) {
              filesToCheck.push(match[1])
            }
          }
        }

        // Check blast scores for each file (limit to first 3 to avoid storms).
        const warnings: string[] = []
        for (const f of filesToCheck.slice(0, 3)) {
          try {
            const resp = await hermesCall("hermes_blast_score", { query: f })
            if (resp.output && resp.output.includes("HIGH")) {
              warnings.push(`⚠️  ${f} — blast score HIGH. Check impact before committing.`)
            }
          } catch { /* skip if unreachable */ }
        }

        if (warnings.length > 0) {
          console.warn("[hermes] " + warnings.join("\n"))
        }
      }
    },

    "experimental.session.compacting": async (_input, output) => {
      // ── Mission context injection ──────────────────────────────────
      if (HERMES_MISSION_ID) {
        try {
          const missionResult = await hermesCall("hermes_mission_status", { mission_id: HERMES_MISSION_ID })
          if (!missionResult.output.startsWith("Hermes unreachable") && !missionResult.output.startsWith("Hermes error")) {
            const parsed = JSON.parse(missionResult.output)
            output.context.push(
              `## Active Mission: ${parsed.mission.title}\n` +
              `**Status**: ${parsed.mission.status}\n` +
              `**Events logged**: ${parsed.log.length}\n` +
              `**Session**: ${SESSION_ID}\n`
            )
          }
        } catch { /* skip */ }

        hermesCall("hermes_mission_event", {
          mission_id: HERMES_MISSION_ID,
          event_type: "session_compact",
          data: { session_id: SESSION_ID, reason: "context_limit" },
        }).catch(() => {})
      }

      try {
        // 1. Recall recent context
        const recallResult = await hermesCall("hermes_recall", { query: "recent work decisions context" })
        if (!recallResult.output.startsWith("Hermes unreachable") && !recallResult.output.startsWith("Hermes error") && !recallResult.output.startsWith("Hermes HTTP")) {
          output.context.push(
            "## Prior Session Context (from Hermes)\n" +
            "The following was recalled from Hermes session memory:\n" +
            recallResult.output
          )
        }

        // 2. Inject top high-blast files summary
        try {
          const blastResult = await hermesCall("hermes_high_blast", { threshold: 5, limit: 5 })
          if (blastResult.output && !blastResult.output.startsWith("Hermes unreachable")) {
            output.context.push(
              "## High-Impact Files (blast score >= 5)\n" +
              "These files have the largest downstream impact. Modify with caution:\n" +
              blastResult.output
            )
          }
        } catch { /* skip */ }

        // 3. Inject unresolved findings count
        try {
          const nextResult = await hermesCall("hermes_quality_next", {})
          if (nextResult.output && !nextResult.output.startsWith("Hermes unreachable")) {
            output.context.push(
              "## Highest-Priority Open Finding\n" +
              nextResult.output
            )
          }
        } catch { /* skip */ }
      } catch {
        // graceful degradation
      }
    },
  }
}
