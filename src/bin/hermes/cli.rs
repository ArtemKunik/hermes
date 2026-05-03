// tools/hermes-engine/src/bin/hermes/cli.rs

pub fn print_usage() {
    eprintln!(
        "hermes \u{2014} token-efficient code navigation\n\n\
         USAGE: hermes <command> [args]\n\n\
         Commands:\n\
           index [--enrich]    Re-index the project; returns busy immediately if another index is active\n\
                               add --enrich to call llm-gateway for richer summaries\n\
           search <query>      Search codebase; returns pointers (no full content)\n\
           fetch <node_id>     Fetch full content for a specific pointer\n\
           recall <query>      Recall prior sessions/decisions and related code context\n\
           fact <type> <text>  Record a decision/learning (types: architecture, decision,\n\
                               learning, constraint, error_pattern, api_contract)\n\
           facts [type]        List active facts, optionally filtered by type\n\
           stats [--since <duration>]  Show token savings (--since: 24h, 7d, 30d, all)\n\
           review <path> [--dim <id>] [--tier <T#>] [--force-accept] [--verbose]  Run LLM quality review; --verbose shows per-file progress\n\
           weight-get <id>     Show weight record for a node (default 1.0 if absent)\n\
           weight-set <id> <d> Adjust node weight by delta (+ reinforce / - decay)\n\
           weight-list         List all nodes with explicit weight entries\n\
           nodes-weight-list   List ALL graph nodes with weights (default 1.0 if not adjusted);\n\
                               use this instead of weight-list for AD-04 consolidation\n\
           delete-node <id>    Hard-delete a node and all its index data (archive first!)\n\
           scan-duplicates <signature>  Find semantically similar symbols by embedding\n\
           prepare-commit-message <subject> [--task <id>] [--decision <path>]\n\
                               [--session <path>] [--docs <csv>] [--pipeline <csv>]\n\
                               [--changes <csv>] [--body <text>]\n\
           validate-env <VAR>  Check environment variable name against registry (TRACK-040)\n\
            validate-symbols <sym1> [sym2 ...]  Ensure each symbol exists in the code graph\n\
            list-tracks [--status <state>]     List conductor tracks with normalized status\n\
            resume-track <id>|--auto [--status <state>]\n\
                                Build a read-only resume brief for one unfinished track\n\
            lint-architecture   Scan codebase for architecture violations (TRACK-045)\n\
                                Flags: --scope <path>, --severity-min error|warning|info,\n\
                                       --rules LAYER-001,SIZE-001,...\n\
            heal-violations     TRACK-048 phase 1 constrained healing candidate generator\n\
                       Flags: --scope <path>, --severity-min error|warning|info,\n\
                           --rules SAFETY-001,SAFETY-002, --max-items <n>, --apply\n\
           search-misses [--since <Nd>] [--top <N>]\n\
                               Post-mortem view of zero-result searches (all-time or last N days)\n\
           review <path> [--dim QD-XX] [--tier T4]  Quality Lens: LLM review of files\n\
           score [--module <name>] [--trend]         Quality score per module and project\n\
           next-review [--module <name>]             Highest-priority open finding\n\
           resolve-review --id <id>                 Mark finding resolved, recompute score\n\
           wontfix-review --id <id> --reason <text> Halve penalty (acknowledge, not hide)\n\
           --stdio             Run as MCP JSON-RPC 2.0 stdio server (for VS Code Copilot)\n\n\
         Env vars:\n\
           HERMES_PROJECT_ROOT             Root directory to index (default: cwd)\n\
           HERMES_DB_PATH                  SQLite DB path (default: <project_root>/.hermes.db)\n\
           HERMES_AUTO_INDEX_INTERVAL_SECS Re-index interval when running as MCP server\n\
                                           (default: 300 = 5 min; 0 = disabled)\n\
           HERMES_DB_BUSY_TIMEOUT           Busy timeout (seconds) for SQLite.\n\
                                           Helps avoid \"database is locked\" errors. Default 30s."
    );
}

/// Extract the value of a named flag (e.g. --dim QD-01) from the arg list.
pub fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.clone())
}
