use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::Path;
use tracing::{error, info};

use crate::{
    engine_cache::{EngineCache, parse_project_registry, spawn_auto_reindex},
    mcp_tools::dispatch,
    HermesEngine,
};

pub fn run(engine: &HermesEngine, project_root: &Path) -> Result<()> {
    spawn_auto_reindex(engine.clone(), project_root.to_path_buf());
    let registry = parse_project_registry();
    if !registry.is_empty() {
        let names: Vec<&str> = registry.iter().map(|e| e.project_id.as_str()).collect();
        info!("[hermes] registered projects: {}", names.join(", "));
    }
    let cache = EngineCache::new(engine.clone(), project_root.to_path_buf(), registry);

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_error(&mut out, &Value::Null, -32700, &format!("parse error: {e}"))?;
                continue;
            }
        };

        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg["method"].as_str().unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        if method.starts_with("notifications/") {
            continue;
        }

        let result = dispatch(&cache, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, &id, payload)?,
            Err(e) => write_error(&mut out, &id, -32603, &e.to_string())?,
        }
    }
    Ok(())
}

/// Serve the MCP JSON-RPC API over plain HTTP on the given port.
/// Accepts POST /api/mcp with a JSON-RPC 2.0 body and returns a JSON-RPC 2.0 response.
pub fn run_http(engine: &HermesEngine, project_root: &Path, port: u16) -> Result<()> {
    use std::sync::Arc;
    spawn_auto_reindex(engine.clone(), project_root.to_path_buf());
    let registry = parse_project_registry();
    if !registry.is_empty() {
        let names: Vec<&str> = registry.iter().map(|e| e.project_id.as_str()).collect();
        info!("[hermes] registered projects: {}", names.join(", "));
    }
    let cache = Arc::new(EngineCache::new(
        engine.clone(),
        project_root.to_path_buf(),
        registry,
    ));

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let addr = format!("[::]:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("[hermes] HTTP MCP listening on http://localhost:{port}/api/mcp (dual-stack)");
                l
            }
            Err(_) => {
                let addr4 = format!("0.0.0.0:{port}");
                let l = tokio::net::TcpListener::bind(&addr4).await?;
                info!("[hermes] HTTP MCP listening on http://localhost:{port}/api/mcp (IPv4 only)");
                l
            }
        };
        loop {
            let (stream, _peer) = listener.accept().await?;
            let cache = Arc::clone(&cache);
            tokio::spawn(async move {
                if let Err(e) = handle_http_conn(stream, cache).await {
                    error!("[hermes] http conn error: {e}");
                }
            });
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

async fn handle_http_conn(
    stream: tokio::net::TcpStream,
    cache: std::sync::Arc<EngineCache>,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;

    // Read until we have the full headers (\r\n\r\n)
    let mut raw = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&tmp[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if raw.len() > 1_048_576 {
            anyhow::bail!("headers too large");
        }
    };

    let headers_text = std::str::from_utf8(&raw[..header_end]).unwrap_or("");
    let first_line = headers_text.lines().next().unwrap_or("");

    // Handle CORS preflight
    if first_line.starts_with("OPTIONS") {
        let resp = b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n";
        stream.write_all(resp).await?;
        return Ok(());
    }

    // Parse Content-Length
    let content_length: usize = headers_text
        .lines()
        .find(|l| l.to_lowercase().starts_with("content-length:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    // Read body (may have started after headers)
    let body_start = header_end + 4;
    let mut body = raw[body_start..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    let msg: Value = serde_json::from_slice(&body)
        .map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg["method"].as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // Notifications get an empty 204
    if method.starts_with("notifications/") {
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
            .await?;
        return Ok(());
    }

    // Run dispatch in a blocking thread — the neural embedding client uses
    // reqwest::blocking which must not be called from within an async context.
    let cache_clone = cache.clone();
    let method_owned = method.to_string();
    let params_owned = params.clone();
    let dispatch_result = tokio::task::spawn_blocking(move || {
        dispatch(&cache_clone, &method_owned, &params_owned)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panic: {e}"))?;

    let response_body = match dispatch_result {
        Ok(payload) => serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": payload,
        }))?,
        Err(e) => serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32603, "message": e.to_string()},
        }))?,
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        response_body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&response_body).await?;
    Ok(())
}

fn write_ok(out: &mut impl Write, id: &Value, result: Value) -> Result<()> {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(out, "{}", serde_json::to_string(&envelope)?)?;
    out.flush()?;
    Ok(())
}

fn write_error(out: &mut impl Write, id: &Value, code: i32, message: &str) -> Result<()> {
    let envelope = json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    });
    writeln!(out, "{}", serde_json::to_string(&envelope)?)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_ok_produces_jsonrpc_envelope() {
        let mut buf = Vec::new();
        write_ok(&mut buf, &json!(42), json!({"data": "hello"})).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["result"]["data"], "hello");
    }

    #[test]
    fn write_ok_with_null_id() {
        let mut buf = Vec::new();
        write_ok(&mut buf, &Value::Null, json!({"status": "ok"})).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert!(parsed["id"].is_null());
        assert_eq!(parsed["result"]["status"], "ok");
    }

    #[test]
    fn write_error_produces_error_envelope() {
        let mut buf = Vec::new();
        write_error(&mut buf, &json!(1), -32603, "internal error").unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["error"]["code"], -32603);
        assert_eq!(parsed["error"]["message"], "internal error");
    }
}
