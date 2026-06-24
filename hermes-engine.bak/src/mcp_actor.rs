use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

#[path = "mcp_actor_dispatch.rs"]
mod mcp_actor_dispatch;

use crate::{mcp_tools, HermesEngine};
use mcp_actor_dispatch::{execute_tool_call, is_read_only_tool};

#[derive(Clone)]
pub struct ToolActor {
    tx: Sender<ActorMessage>,
    pending_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    circuit_breaker: crate::tool_circuit_breaker::ToolCircuitBreaker,
}

enum ActorMessage {
    ToolCall {
        name: String,
        args: Value,
        reply: Sender<Result<String>>,
    },
    AutoIndex,
}

impl ToolActor {
    pub fn start(engine: HermesEngine, project_root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<ActorMessage>();
        let pending_calls = engine.pending_calls();
        let circuit_breaker = engine.tool_circuit_breaker();
        std::thread::spawn(move || actor_loop(engine, project_root, rx));
        Self {
            tx,
            pending_calls,
            circuit_breaker,
        }
    }

    pub fn call_tool(&self, name: &str, args: &Value) -> Result<String> {
        if let Err(e) = self.circuit_breaker.check(name) {
            eprintln!("[hermes:circuit] REJECTED name={name} error={e}");
            return Err(anyhow::anyhow!(e));
        }

        let (reply_tx, reply_rx) = mpsc::channel::<Result<String>>();
        self.pending_calls.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(ActorMessage::ToolCall {
                name: name.to_string(),
                args: args.clone(),
                reply: reply_tx,
            })
            .map_err(|e| {
                self.pending_calls.fetch_sub(1, Ordering::Relaxed);
                anyhow::anyhow!("tool actor send failed: {e}")
            })?;

        let timeout_ms = crate::tool_runtime::resolve_tool_timeout_ms();
        let result = reply_rx.recv_timeout(Duration::from_millis(timeout_ms));
        self.pending_calls.fetch_sub(1, Ordering::Relaxed);

        match result {
            Ok(result) => {
                self.circuit_breaker.record_success(name);
                result
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!("[hermes:TIMEOUT] name={name} elapsed_ms={timeout_ms}");
                self.circuit_breaker.record_timeout(name);
                Err(anyhow::anyhow!(
                    r#"{{"error":"tool_timeout","tool":"{name}","elapsed_ms":{timeout_ms}}}"#
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[hermes:ERROR] name={name} error=channel_disconnected");
                Err(anyhow::anyhow!("tool actor receive failed: channel disconnected"))
            }
        }
    }

    pub fn enqueue_auto_index(&self) -> Result<()> {
        self.tx
            .send(ActorMessage::AutoIndex)
            .map_err(|e| anyhow::anyhow!("auto-index enqueue failed: {e}"))
    }
}

fn actor_loop(engine: HermesEngine, project_root: PathBuf, rx: Receiver<ActorMessage>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            ActorMessage::ToolCall { name, args, reply } => {
                let engine = engine.clone();
                let project_root = project_root.clone();
                thread::spawn(move || {
                    crate::tool_runtime::log_tool_start(&name, &args);
                    let started_at = Instant::now();
                    let payload_bytes = args.to_string().len();
                    #[cfg(test)]
                    maybe_sleep_for_test_tool_delay();

                    // TRACK-066: Isolate read-only tool calls by using a fresh read-only connection
                    // when possible. This prevents diagnostic/search tools from contending
                    // with the write lock held by the auto-indexer.
                    let result = match is_read_only_tool(&name) {
                        true => {
                            match engine.diagnostic_db() {
                                Ok(db) => {
                                    let conn = db.lock().unwrap_or_else(|e: PoisonError<MutexGuard<Connection>>| e.into_inner());
                                    execute_tool_call(&engine, &conn, &project_root, &name, &args)
                                }
                                Err(e) => Err(anyhow::anyhow!("failed to open diagnostic connection: {e}")),
                            }
                        }
                        false => {
                            let conn = engine.db().lock().unwrap_or_else(|e: PoisonError<MutexGuard<Connection>>| e.into_inner());
                            execute_tool_call(&engine, &conn, &project_root, &name, &args)
                        }
                    };

                    let success = result.is_ok();
                    crate::tool_runtime::log_tool_call(
                        &name,
                        payload_bytes,
                        started_at.elapsed().as_millis(),
                        success,
                    );
                    let _ = reply.send(result);
                });
            }
            ActorMessage::AutoIndex => {
                let engine = engine.clone();
                let project_root = project_root.clone();
                thread::spawn(move || {
                    if let Err(err) = mcp_tools::tool_index(&engine, &project_root) {
                        eprintln!("[hermes] auto-index actor execution failed: {err}");
                    }
                });
            }
        }
    }
}

#[cfg(test)]
fn maybe_sleep_for_test_tool_delay() {
    if let Ok(delay_ms) = std::env::var("HERMES_TEST_TOOL_DELAY_MS") {
        if let Ok(ms) = delay_ms.parse::<u64>() {
            if ms > 0 {
                thread::sleep(Duration::from_millis(ms));
            }
        }
    }
}

#[cfg(not(test))]
fn maybe_sleep_for_test_tool_delay() {}

 
 
 
