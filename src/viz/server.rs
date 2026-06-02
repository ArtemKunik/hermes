use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tiny_http::{Response, Server as HttpServer, Method, Header};

use crate::HermesEngine;

pub const DEFAULT_VIZ_PORT: u16 = 8080;

const INDEX_HTML: &str = include_str!("static/index.html");

pub fn run_viz_server(engine: &HermesEngine, _project_root: &Path, port: u16) -> Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let server = HttpServer::http(&addr)
        .map_err(|e| anyhow::anyhow!("Cannot bind {addr}: {e}"))?;

    eprintln!("Hermes viz server running on http://localhost:{port}");
    eprintln!("  Graph view:   http://localhost:{port}");
    eprintln!("  API:          http://localhost:{port}/api/graph");
    eprintln!("  Ctrl+C to stop");

    let running = Arc::new(AtomicBool::new(true));

    while running.load(Ordering::SeqCst) {
        match server.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(Some(request)) => {
                let response = handle_request(engine, &request);
                let _ = request.respond(response);
            }
            Ok(None) => { /* timeout */ }
            Err(e) => {
                eprintln!("Server error: {e}");
                break;
            }
        }
    }

    eprintln!("Viz server stopped.");
    Ok(())
}

fn handle_request(
    engine: &HermesEngine,
    request: &tiny_http::Request,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let url = request.url().to_string();

    if request.method() != &Method::Get {
        return json(405, serde_json::json!({"error": "method not allowed"}));
    }

    match url.as_str() {
        "/" | "/index.html" => html(INDEX_HTML),
        "/api/graph" => api_result(crate::viz::api::get_graph_json(engine)),
        path if path == "/api/blast" || path.starts_with("/api/blast?") => {
            let threshold = request.url().split('?')
                .nth(1).unwrap_or("")
                .split('&')
                .find_map(|p| p.strip_prefix("threshold="))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0);
            api_result(crate::viz::api::get_blast_json(engine, threshold))
        }
        path if path.starts_with("/api/symbols/") => {
            let file_path = &path["/api/symbols/".len()..];
            api_result(crate::viz::api::get_symbols_json(engine, file_path))
        }
        _ => json(404, serde_json::json!({"error": "not found"})),
    }
}

fn api_result(r: Result<serde_json::Value>) -> Response<std::io::Cursor<Vec<u8>>> {
    match r {
        Ok(v) => json(200, v),
        Err(e) => json(500, serde_json::json!({"error": format!("{e}")})),
    }
}

fn html<'a>(body: &'a str) -> Response<std::io::Cursor<Vec<u8>>> {
    let h: Header = "Content-Type: text/html; charset=utf-8".parse().unwrap();
    Response::from_data(body.as_bytes().to_vec()).with_header(h)
}

fn json<'a>(status: u16, value: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
    let h: Header = "Content-Type: application/json; charset=utf-8".parse().unwrap();
    Response::from_data(body.into_bytes())
        .with_status_code(status)
        .with_header(h)
}
