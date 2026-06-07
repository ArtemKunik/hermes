// TRACK-053: Per-tool circuit breaker — prevents cascading failures when a
// tool times out repeatedly. Default state is CLOSED (healthy).  After
// HERMES_TOOL_FAILURE_THRESHOLD (default 3) consecutive timeouts the circuit
// opens.  After HERMES_TOOL_COOLDOWN_SEC (default 300) seconds in OPEN state,
// the circuit transitions to HALF_OPEN, allowing one probe call through.  A
// successful probe closes the circuit; another timeout re-opens it.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn read_failure_threshold() -> usize {
    std::env::var("HERMES_TOOL_FAILURE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

fn read_cooldown_secs() -> u64 {
    std::env::var("HERMES_TOOL_COOLDOWN_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300)
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

struct ToolState {
    circuit: CircuitState,
    timeout_count: usize,
    opened_at: Option<Instant>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            circuit: CircuitState::Closed,
            timeout_count: 0,
            opened_at: None,
        }
    }
}

#[derive(Clone)]
pub struct ToolCircuitBreaker {
    state: Arc<Mutex<HashMap<String, ToolState>>>,
    cooldown_secs: u64,
    failure_threshold: usize,
}

impl ToolCircuitBreaker {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            cooldown_secs: read_cooldown_secs(),
            failure_threshold: read_failure_threshold(),
        }
    }

    #[cfg(test)]
    pub fn new_with_config(cooldown_secs: u64, failure_threshold: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            cooldown_secs,
            failure_threshold,
        }
    }

    /// Returns `Err` when the circuit is OPEN (and cooldown has not elapsed).
    /// Transparently transitions OPEN → HALF_OPEN once the cooldown expires.
    pub fn check(&self, name: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(name.to_string()).or_default();
        match entry.circuit {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
            CircuitState::Open => {
                let elapsed = entry.opened_at.map_or(u64::MAX, |t| t.elapsed().as_secs());
                if elapsed >= self.cooldown_secs {
                    entry.circuit = CircuitState::HalfOpen;
                    eprintln!("[hermes:circuit] tool={name} state=HALF_OPEN");
                    Ok(())
                } else {
                    Err(format!(
                        "circuit OPEN for tool '{name}' — blocked for {}s more",
                        self.cooldown_secs.saturating_sub(elapsed)
                    ))
                }
            }
        }
    }

    /// Record a successful tool call; resets the timeout counter and closes
    /// the circuit.
    pub fn record_success(&self, name: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = state.get_mut(name) {
            if entry.circuit != CircuitState::Closed {
                eprintln!("[hermes:circuit] tool={name} state=CLOSED");
            }
            entry.circuit = CircuitState::Closed;
            entry.timeout_count = 0;
            entry.opened_at = None;
        }
    }

    /// Record a timed-out tool call; opens the circuit after the failure threshold.
    /// If the circuit was HALF_OPEN, a single failure re-opens it immediately.
    pub fn record_timeout(&self, name: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state.entry(name.to_string()).or_default();
        match entry.circuit {
            CircuitState::Closed => {
                entry.timeout_count += 1;
                if entry.timeout_count >= self.failure_threshold {
                    entry.circuit = CircuitState::Open;
                    entry.opened_at = Some(Instant::now());
                    eprintln!(
                        "[hermes:circuit] tool={name} state=OPEN failures={}",
                        entry.timeout_count
                    );
                }
            }
            CircuitState::HalfOpen | CircuitState::Open => {
                // If we're already Open or HalfOpen and we fail, we stay/return to Open
                // and reset the cooldown timer.
                entry.circuit = CircuitState::Open;
                entry.opened_at = Some(Instant::now());
                entry.timeout_count = self.failure_threshold;
                eprintln!("[hermes:circuit] tool={name} state=OPEN (reset cooldown)");
            }
        }
    }

    /// Returns a JSON object mapping each tool name to its circuit state
    /// ("CLOSED", "OPEN", or "HALF_OPEN").  Tools with no recorded activity are CLOSED.
    pub fn snapshot_for_tools(&self, tool_names: &[String]) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cooldown = self.cooldown_secs;
        let mut map = serde_json::Map::new();
        for name in tool_names {
            let label = match state.get_mut(name.as_str()) {
                None => "CLOSED",
                Some(entry) => match entry.circuit {
                    CircuitState::Closed => "CLOSED",
                    CircuitState::HalfOpen => "HALF_OPEN",
                    CircuitState::Open => {
                        // Reflect cooldown expiry in snapshot too.
                        let elapsed = entry.opened_at.map_or(u64::MAX, |t| t.elapsed().as_secs());
                        if elapsed >= cooldown {
                            entry.circuit = CircuitState::HalfOpen;
                            "HALF_OPEN"
                        } else {
                            "OPEN"
                        }
                    }
                },
            };
            map.insert(name.clone(), json!(label));
        }
        Value::Object(map)
    }
}

impl Default for ToolCircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: usize = 3;
    const LONG_COOLDOWN: u64 = 9999;

    fn trip(cb: &ToolCircuitBreaker, name: &str) {
        for _ in 0..THRESHOLD {
            cb.record_timeout(name);
        }
    }

    #[test]
    fn test_circuit_starts_closed() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        assert!(cb.check("hermes_search").is_ok());
    }

    #[test]
    fn test_circuit_opens_after_threshold() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        trip(&cb, "hermes_search");
        assert!(cb.check("hermes_search").is_err());
    }

    #[test]
    fn test_circuit_closes_on_success() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        trip(&cb, "hermes_search");
        assert!(cb.check("hermes_search").is_err());
        cb.record_success("hermes_search");
        assert!(cb.check("hermes_search").is_ok());
    }

    #[test]
    fn test_circuit_half_opens_after_cooldown() {
        let cb = ToolCircuitBreaker::new_with_config(0, THRESHOLD);
        trip(&cb, "hermes_index");
        // With cooldown=0 the circuit should immediately allow check through as HALF_OPEN.
        assert!(cb.check("hermes_index").is_ok());
    }

    #[test]
    fn test_snapshot_returns_closed_for_healthy_tools() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        let tools = vec!["hermes_recall".to_string(), "hermes_search".to_string()];
        let snap = cb.snapshot_for_tools(&tools);
        assert_eq!(snap["hermes_recall"].as_str(), Some("CLOSED"));
        assert_eq!(snap["hermes_search"].as_str(), Some("CLOSED"));
    }

    #[test]
    fn test_snapshot_returns_open_for_tripped_tool() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        trip(&cb, "hermes_fetch");
        let tools = vec!["hermes_fetch".to_string(), "hermes_search".to_string()];
        let snap = cb.snapshot_for_tools(&tools);
        assert_eq!(snap["hermes_fetch"].as_str(), Some("OPEN"));
        assert_eq!(snap["hermes_search"].as_str(), Some("CLOSED"));
    }

    #[test]
    fn test_circuit_half_open_failure_resets_cooldown() {
        let cb = ToolCircuitBreaker::new_with_config(LONG_COOLDOWN, THRESHOLD);
        trip(&cb, "hermes_search");
        assert!(cb.check("hermes_search").is_err());

        {
            let mut state = cb.state.lock().unwrap();
            state.get_mut("hermes_search").unwrap().circuit = CircuitState::HalfOpen;
        }

        // Failure in HALF_OPEN should reset to OPEN and set new opened_at
        let before = Instant::now();
        cb.record_timeout("hermes_search");

        let state = cb.state.lock().unwrap();
        let entry = state.get("hermes_search").unwrap();
        assert_eq!(entry.circuit, CircuitState::Open);
        assert!(entry.opened_at.unwrap() >= before);
    }
}
