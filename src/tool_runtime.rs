// TRACK-053: Tool runtime helpers — timeout resolution and call-site telemetry.
use serde_json::Value;

/// Returns the per-tool call timeout in milliseconds.
/// Reads `HERMES_TOOL_TIMEOUT_MS` from the environment; defaults to 15 000 ms.
pub fn resolve_tool_timeout_ms() -> u64 {
    std::env::var("HERMES_TOOL_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(15_000)
}

/// Emit a structured log line when a tool invocation begins.
pub fn log_tool_start(name: &str, args: &Value) {
    let args_str = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    eprintln!("[hermes:start] name={name} args={args_str}");
}

/// Emit a structured log line for every tool invocation.
/// Logs `[hermes:SLOW]` when elapsed exceeds `HERMES_TOOL_SLOW_MS` (default 5000).
/// Intentionally lightweight (eprintln) so it never blocks the actor thread.
pub fn log_tool_call(name: &str, payload_bytes: usize, elapsed_ms: u128, success: bool) {
    let slow_threshold_ms = std::env::var("HERMES_TOOL_SLOW_MS")
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(5_000);

    let status = if success { "success" } else { "error" };

    if elapsed_ms > slow_threshold_ms {
        eprintln!(
            "[hermes:SLOW] name={name} duration_ms={elapsed_ms} payload_bytes={payload_bytes} status={status}"
        );
    } else {
        eprintln!(
            "[hermes:tool] name={name} duration_ms={elapsed_ms} payload_bytes={payload_bytes} status={status}"
        );
    }
}
