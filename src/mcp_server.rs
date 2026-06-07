// tools/hermes-engine/src/mcp_server.rs
//
// Native MCP JSON-RPC 2.0 stdio server.
// Protocol: auto-detects either MCP framed stdio (Content-Length) or
// newline-delimited JSON for backward compatibility.
//
// Supported methods: initialize, notifications/initialized, tools/list, tools/call
// Tools: hermes_search, hermes_fetch, hermes_index, hermes_stats, hermes_fact, hermes_facts, hermes_remember, hermes_write_decision, hermes_log_incident, hermes_resolve_incident, hermes_query_incidents, hermes_write_kb_article, hermes_search_kb

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use crate::{
    mcp_actor::ToolActor,
    mcp_tool_schemas,
    mcp_wire::{self, WireMode},
    HermesEngine,
};

// ---------------------------------------------------------------------------
// Auto-reindex background thread
// ---------------------------------------------------------------------------

/// Spawn a background thread that periodically re-indexes the project.
///
/// Interval is controlled by `HERMES_AUTO_INDEX_INTERVAL_SECS` (default: 300).
/// Set to `0` to disable.
/// Logs progress to stderr so it does not interfere with the MCP stdio protocol.
fn spawn_auto_reindex(actor: ToolActor) {
    let interval_secs = std::env::var("HERMES_AUTO_INDEX_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);

    if interval_secs == 0 {
        eprintln!("[hermes] auto-reindex disabled (HERMES_AUTO_INDEX_INTERVAL_SECS=0)");
        return;
    }

    std::thread::spawn(move || {
        eprintln!(
            "[hermes] auto-reindex thread started (interval={}s)",
            interval_secs
        );
        run_auto_index_cycle(&actor, "startup");

        loop {
            std::thread::sleep(std::time::Duration::from_secs(interval_secs));
            run_auto_index_cycle(&actor, "auto-reindex");
        }
    });
}

fn run_auto_index_cycle(actor: &ToolActor, phase: &str) {
    if let Err(err) = actor.enqueue_auto_index() {
        eprintln!("[hermes] {phase} skipped: failed to enqueue auto-index: {err}");
    }
}

fn spawn_heartbeat(engine: HermesEngine) {
    let interval_secs = std::env::var("HERMES_HEARTBEAT_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10);

    if interval_secs == 0 {
        return;
    }

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(interval_secs));
        let pending_calls = engine.pending_calls().load(Ordering::Relaxed);
        eprintln!("[hermes:heartbeat] alive pending_calls={pending_calls}");
    });
}

// ---------------------------------------------------------------------------
// Periodic session checkpoint thread
// ---------------------------------------------------------------------------

/// Spawn a background thread that writes a lightweight session checkpoint file
/// every `HERMES_SESSION_CHECKPOINT_INTERVAL_SECS` seconds (default: 600).
/// Background checkpoints intentionally skip immediate ingest so they do not
/// contend with live MCP tool calls on the shared engine connection. The next
/// normal auto-index cycle will pick them up. Set to `0` to disable.
fn spawn_auto_session_checkpoint(engine: HermesEngine, project_root: std::path::PathBuf) {
    let interval_secs = std::env::var("HERMES_SESSION_CHECKPOINT_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(600);

    if interval_secs == 0 {
        eprintln!(
            "[hermes] session checkpoint disabled (HERMES_SESSION_CHECKPOINT_INTERVAL_SECS=0)"
        );
        return;
    }

    std::thread::spawn(move || {
        eprintln!(
            "[hermes] session checkpoint thread started (interval={}s)",
            interval_secs
        );
        loop {
            std::thread::sleep(Duration::from_secs(interval_secs));
            if let Err(e) =
                crate::mcp_memory::write_session_checkpoint(&engine, &project_root, false)
            {
                eprintln!("[hermes] session checkpoint failed: {e}");
            } else {
                eprintln!("[hermes] session checkpoint written");
            }
        }
    });
}

/// Run the MCP stdio server until stdin is closed.
pub fn run(engine: &HermesEngine, project_root: &Path) -> Result<()> {
    let actor = ToolActor::start(engine.clone(), project_root.to_path_buf());
    eprintln!(
        "[hermes] mcp server starting pid={} session_id={} project_root={}",
        std::process::id(),
        engine.session_id(),
        project_root.display()
    );
    spawn_heartbeat(engine.clone());

    // TRACK-045: Prefer file-watcher over timer-based auto-reindex when enabled.
    let use_watcher = std::env::var("HERMES_USE_FILE_WATCHER")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    if use_watcher {
        match crate::watcher::start_file_watcher(actor.clone(), project_root) {
            Ok(()) => eprintln!("[hermes] file-watcher mode active (replaces timer reindex)"),
            Err(e) => {
                eprintln!("[hermes] file-watcher failed to start ({e}), falling back to timer");
                spawn_auto_reindex(actor.clone());
            }
        }
    } else {
        // Start background auto-reindex thread before entering the stdin loop.
        spawn_auto_reindex(actor.clone());
    }

    // Start background session checkpoint thread.
    spawn_auto_session_checkpoint(engine.clone(), project_root.to_path_buf());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut out = stdout.lock();
    let mut wire_mode = WireMode::Auto;

    loop {
        let Some(raw) = mcp_wire::read_next_message(&mut reader, &mut wire_mode)? else {
            break;
        };

        let msg: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                write_error(
                    &mut out,
                    wire_mode,
                    &Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                )?;
                continue;
            }
        };

        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg["method"].as_str().unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Client notifications carry no id and require no response.
        if method.starts_with("notifications/") {
            continue;
        }

        let result = dispatch_with_lock_retry(&actor, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, wire_mode, &id, payload)?,
            Err(e) => write_error(&mut out, wire_mode, &id, -32603, &e.to_string())?,
        }
    }
    eprintln!(
        "[hermes] mcp server stopping pid={} session_id={}",
        std::process::id(),
        engine.session_id()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn dispatch(actor: &ToolActor, method: &str, params: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list(params)),
        "tools/call" => handle_tool_call(actor, params),
        other => anyhow::bail!("unknown method: {other}"),
    }
}

fn dispatch_with_lock_retry(actor: &ToolActor, method: &str, params: &Value) -> Result<Value> {
    let retries = crate::retry::lock_retry_budget();
    let mut attempt = 0usize;

    loop {
        let call_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch(actor, method, params)
        }));
        match call_result {
            Ok(Ok(value)) => {
                if attempt > 0 {
                    eprintln!("[hermes] sqlite lock recovered method={method} attempts={attempt}");
                }
                return Ok(value);
            }
            Ok(Err(err)) => {
                let err_text = err.to_string();
                let is_locked = crate::retry::is_database_locked_message(&err_text);
                if attempt >= retries || !is_locked {
                    eprintln!(
                        "[hermes] method failed method={method} attempts={} lock_related={} error={err_text}",
                        attempt + 1,
                        is_locked
                    );
                    return Err(err);
                }
                let delay_ms = crate::retry::lock_retry_delay_ms(attempt);
                eprintln!(
                    "[hermes] sqlite lock detected; retrying method={method} attempt={}/{} delay={}ms",
                    attempt + 1,
                    retries,
                    delay_ms
                );
                thread::sleep(Duration::from_millis(delay_ms));
                attempt += 1;
            }
            Err(_) => {
                eprintln!("[hermes] tool handler panicked method={method}");
                return Err(anyhow::anyhow!(
                    "tool handler panicked while processing {method}"
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MCP method handlers
// ---------------------------------------------------------------------------

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "Hermes", "version": env!("CARGO_PKG_VERSION") }
    })
}

pub(crate) fn handle_tools_list(params: &Value) -> Value {
    let payload = if let Ok(profile) = std::env::var("HERMES_TOOL_PROFILE") {
        // Profile-based filtering: core (8 tools) | standard (16 tools) | full (all, default)
        crate::tool_router::tools_payload_for_profile(&profile)
    } else if routing_enabled() {
        let intent = params["intent"].as_str().unwrap_or("all");
        crate::tool_router::tools_payload_for_intent(intent)
    } else {
        mcp_tool_schemas::tools_list()
    };
    let serialized = payload.to_string();
    let tool_count = payload["tools"].as_array().map_or(0_usize, |arr| arr.len());
    eprintln!(
        "[hermes:schema] tools_list served tool_count={tool_count} schema_bytes={}",
        serialized.len()
    );
    payload
}

fn handle_tool_call(actor: &ToolActor, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];
    anyhow::ensure!(!name.is_empty(), "tools/call requires 'name'");
    let text = actor.call_tool(name, args)?;

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn routing_enabled() -> bool {
    std::env::var("HERMES_TOOL_ROUTING")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Wire helpers — write JSON-RPC envelope to stdout
// ---------------------------------------------------------------------------

fn write_ok(out: &mut impl Write, mode: WireMode, id: &Value, result: Value) -> Result<()> {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    mcp_wire::write_json(out, mode, &envelope)?;
    Ok(())
}

fn write_error(
    out: &mut impl Write,
    mode: WireMode,
    id: &Value,
    code: i32,
    message: &str,
) -> Result<()> {
    let envelope = json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    });
    mcp_wire::write_json(out, mode, &envelope)?;
    Ok(())
}
