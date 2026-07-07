// tools/hermes-engine/src/bin/hermes/main.rs
mod cli;
mod cli_runtime;
mod handlers;
mod handlers_advanced;

use anyhow::{bail, Result};
use hermes_engine::{mcp_quality, mcp_server, mcp_tools, mcp_tools_validation, HermesEngine};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::{parse_flag, print_usage};
use crate::cli_runtime::open_engine;
use crate::handlers::*;
use crate::handlers_advanced::{
    cmd_heal_violations, cmd_lint_architecture, cmd_prepare_commit_message,
    cmd_validate_commit_context,
};

/// Spawn the Hermes HTTP API on a dedicated thread + tokio runtime.
///
/// The HTTP API lets external services (e.g. mastermind daemon) write to the
/// same System 2 store without going through MCP. Disabled when
/// `HERMES_HTTP_DISABLED=1`.
fn spawn_http_api(engine: HermesEngine) {
    if env::var("HERMES_HTTP_DISABLED").ok().as_deref() == Some("1") {
        eprintln!("[hermes] http api disabled (HERMES_HTTP_DISABLED=1)");
        return;
    }
    let port = hermes_engine::http_api::resolve_http_port();
    let engine_arc = Arc::new(engine);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("[hermes] http api: failed to build tokio runtime: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let app = hermes_engine::http_api::build_router(engine_arc);
            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    eprintln!("[hermes] http api listening on {addr}");
                    if let Err(e) = axum::serve(listener, app.into_make_service()).await {
                        eprintln!("[hermes] http api server error: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("[hermes] http api: failed to bind {addr}: {e}");
                }
            }
        });
    });
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let command = args[1].as_str();
    if command == "index" && env::var("HERMES_DB_BUSY_TIMEOUT").is_err() {
        env::set_var("HERMES_DB_BUSY_TIMEOUT", "1");
    }

    let (engine, project_root) = match open_engine(command) {
        Ok(v) => v,
        Err(err)
            if command == "index"
                && hermes_engine::retry::is_database_locked_message(&err.to_string()) =>
        {
            print_non_blocking_busy_index("database is locked");
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    // MCP stdio mode: VS Code spawns us directly as an MCP server.
    if command == "--stdio" {
        spawn_http_api(engine.clone());
        return mcp_server::run(&engine, &project_root);
    }

    match command {
        "index" => {
            // --memory flag indexes memory/ .md files into Qdrant semantic_memory collection.
            if args.iter().any(|a| a == "--memory") {
                let memory_root = project_root.join("memory");
                let rt = tokio::runtime::Runtime::new()?;
                let indexer = hermes_engine::memory_indexer::MemoryIndexer::new()?;
                let stats = rt.block_on(indexer.run(&memory_root))?;
                let output = serde_json::json!({
                    "total_chunks": stats.total_chunks,
                    "upserted": stats.upserted,
                    "skipped": stats.skipped,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }
            // Optional --enrich flag triggers LLM enrichment via llm-gateway-rust.
            let enrich = args.iter().any(|a| a == "--enrich");
            cmd_index(&engine, &project_root, enrich)
        }
        "search" => {
            let query = args.get(2).map(String::as_str).unwrap_or("");
            if query.is_empty() {
                bail!("usage: hermes search <query>");
            }
            cmd_search(&engine, query)
        }
        "fetch" => {
            let id = args.get(2).map(String::as_str).unwrap_or("");
            if id.is_empty() {
                bail!("usage: hermes fetch <node_id>");
            }
            cmd_fetch(&engine, id)
        }
        "recall" => {
            let query = args.get(2).map(String::as_str).unwrap_or("");
            if query.is_empty() {
                bail!("usage: hermes recall <query>");
            }
            cmd_recall(&engine, query)
        }
        "fact" => {
            let fact_type = args.get(2).map(String::as_str).unwrap_or("");
            let content = args.get(3).map(String::as_str).unwrap_or("");
            if fact_type.is_empty() || content.is_empty() {
                bail!("usage: hermes fact <type> <content>");
            }
            cmd_add_fact(&engine, fact_type, content)
        }
        "facts" => {
            let filter = args.get(2).map(String::as_str);
            cmd_list_facts(&engine, filter)
        }
        "stats" => {
            // Task 2.3: support optional --since <duration> flag (24h, 7d, 30d, all)
            let since_arg = args.get(2).map(String::as_str);
            cmd_stats(&engine, since_arg)
        }
        "backfill-tokens" => {
            // Retro-fill stored content token counts for existing nodes
            cmd_backfill(&engine)
        }
        "weight-get" => {
            let node_id = args.get(2).map(String::as_str).unwrap_or("");
            if node_id.is_empty() {
                bail!("usage: hermes weight-get <node_id>");
            }
            cmd_weight_get(&engine, node_id)
        }
        "weight-set" => {
            let node_id = args.get(2).map(String::as_str).unwrap_or("");
            let delta_str = args.get(3).map(String::as_str).unwrap_or("");
            if node_id.is_empty() || delta_str.is_empty() {
                bail!("usage: hermes weight-set <node_id> <delta>");
            }
            let delta: f64 = delta_str
                .parse()
                .map_err(|_| anyhow::anyhow!("delta must be a float"))?;
            cmd_weight_set(&engine, node_id, delta)
        }
        "weight-list" => cmd_weight_list(&engine),
        "nodes-weight-list" => cmd_nodes_weight_list(&engine),
        "delete-node" => {
            let node_id = args.get(2).map(String::as_str).unwrap_or("");
            if node_id.is_empty() {
                bail!("usage: hermes delete-node <node_id>");
            }
            cmd_delete_node(&engine, node_id)
        }
        "validate-env" => {
            let var = args.get(2).map(String::as_str).unwrap_or("");
            if var.is_empty() {
                bail!("usage: hermes validate-env <ENV_VAR>");
            }
            let out = mcp_tools_validation::tool_validate_env(&engine, var)?;
            println!("{out}");
            Ok(())
        }
        "validate-symbols" => {
            let symbols: Vec<&str> = if args.len() <= 2 {
                Vec::new()
            } else {
                args[2..].iter().map(|s| s.as_str()).collect()
            };
            if symbols.is_empty() {
                bail!("usage: hermes validate-symbols <sym1> [sym2 ...]");
            }
            let out = mcp_tools_validation::tool_validate_symbols(&engine, &symbols)?;
            println!("{out}");
            Ok(())
        }
        "list-tracks" => {
            let status = parse_flag(&args, "--status");
            cmd_list_tracks(&engine, &project_root, status.as_deref())
        }
        "resume-track" => {
            let auto = args.iter().any(|arg| arg == "--auto");
            let status = parse_flag(&args, "--status");
            let track_id = args
                .get(2)
                .filter(|value| !value.starts_with("--"))
                .map(String::as_str);
            cmd_resume_track(&engine, &project_root, track_id, auto, status.as_deref())
        }
        "scan-duplicates" => {
            let sig = args.get(2).map(String::as_str).unwrap_or("");
            if sig.is_empty() {
                bail!("usage: hermes scan-duplicates <signature>");
            }
            let out = mcp_tools::tool_scan_duplicates(&engine, sig)?;
            println!("{out}");
            Ok(())
        }
        "lint-architecture" => {
            let scope = parse_flag(&args, "--scope");
            let severity = parse_flag(&args, "--severity-min");
            let rules = parse_flag(&args, "--rules");
            cmd_lint_architecture(
                &engine,
                &project_root,
                scope.as_deref(),
                severity.as_deref(),
                rules.as_deref(),
            )
        }
        "heal-violations" => cmd_heal_violations(&engine, &project_root, &args),
        "prepare-commit-message" => cmd_prepare_commit_message(&args),
        "validate-commit-context" => cmd_validate_commit_context(&args),
        "search-misses" => {
            // Optional: --since <Nd> (e.g. 7d) and --top <N>
            let since_days = args
                .iter()
                .position(|a| a == "--since")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| {
                    let s = s.trim_end_matches('d');
                    s.parse::<u64>().ok()
                });
            let top_k = args
                .iter()
                .position(|a| a == "--top")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(10);
            let out = mcp_tools::tool_search_misses(&engine, since_days, top_k)?;
            println!("{out}");
            Ok(())
        }
        "review" => {
            let path = args.get(2).map(String::as_str).unwrap_or("ChartApp");
            let dim = parse_flag(&args, "--dim");
            let tier = parse_flag(&args, "--tier");
            let force_accept = args.iter().any(|a| a == "--force-accept");
            let verbose = args.iter().any(|a| a == "--verbose");
            let a = serde_json::json!({
                "path": path,
                "dim": dim,
                "tier": tier,
                "force_accept": force_accept,
                "verbose": verbose,
            });
            let out = mcp_quality::tool_quality_review(&engine, &project_root, &a)?;
            println!("{out}");
            Ok(())
        }
        "score" => {
            let module = parse_flag(&args, "--module");
            let trend = args.iter().any(|a| a == "--trend");
            let a = serde_json::json!({"module": module, "trend": trend});
            let out = mcp_quality::tool_quality_score(&engine, &project_root, &a)?;
            println!("{out}");
            Ok(())
        }
        "next-review" => {
            let module = parse_flag(&args, "--module");
            let a = serde_json::json!({"module": module});
            let out = mcp_quality::tool_quality_next(&engine, &project_root, &a)?;
            println!("{out}");
            Ok(())
        }
        "resolve-review" => {
            let id = parse_flag(&args, "--id").unwrap_or_default();
            if id.is_empty() {
                bail!("usage: hermes resolve-review --id <id>");
            }
            let a = serde_json::json!({"id": id});
            let out = mcp_quality::tool_quality_resolve(&engine, &project_root, &a)?;
            println!("{out}");
            Ok(())
        }
        "wontfix-review" => {
            let id = parse_flag(&args, "--id").unwrap_or_default();
            let reason = parse_flag(&args, "--reason").unwrap_or_default();
            if id.is_empty() || reason.is_empty() {
                bail!("usage: hermes wontfix-review --id <id> --reason \"<text>\"");
            }
            let a = serde_json::json!({"id": id, "reason": reason});
            let out = mcp_quality::tool_quality_wontfix(&engine, &project_root, &a)?;
            println!("{out}");
            Ok(())
        }
        "quality-baseline" => {
            let out = hermes_engine::mcp_quality_drift::tool_quality_baseline(
                &engine,
                &project_root,
                &serde_json::json!({}),
            )?;
            println!("{out}");
            Ok(())
        }
        "quality-drift" => {
            let out = hermes_engine::mcp_quality_drift::tool_quality_drift(
                &engine,
                &project_root,
                &serde_json::json!({}),
            )?;
            println!("{out}");
            Ok(())
        }
        "inject-symbols" => {
            let path = args
                .iter()
                .position(|a| a == "--path")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .unwrap_or_else(|| project_root.join("AGENTS.md"));
            let all = args.iter().any(|a| a == "--all");
            let budget = args
                .iter()
                .position(|a| a == "--budget")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(2000);
            let conn = engine
                .read_db()
                .lock()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            hermes_engine::symbol_inject::inject_symbols(
                &conn,
                engine.project_id(),
                &path,
                all,
                budget,
            )?;
            println!("Injected symbols into {}", path.display());
            Ok(())
        }
        "install-hook" => {
            let threshold = args
                .iter()
                .position(|a| a == "--threshold")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(10.0);
            let strict = args.iter().any(|a| a == "--strict");
            let remove = args.iter().any(|a| a == "--remove");

            if remove {
                match hermes_engine::hook::remove_hook(&project_root) {
                    Ok(true) => println!("Removed hermes pre-commit hook"),
                    Ok(false) => println!("No hermes pre-commit hook found"),
                    Err(e) => println!("{e}"),
                }
                return Ok(());
            }

            let script = hermes_engine::hook::generate_hook_script(threshold, strict);
            hermes_engine::hook::install_hook(&project_root, &script)?;
            println!("Installed hermes pre-commit hook (threshold={threshold}, strict={strict})");
            Ok(())
        }
        "serve" => {
            let port = parse_flag(&args, "--port")
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(hermes_engine::viz::server::DEFAULT_VIZ_PORT);
            cmd_viz(&engine, &project_root, port)
        }
        unknown => {
            print_usage();
            bail!("unknown command: {unknown}");
        }
    }
}
