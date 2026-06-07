use hermes_engine::{mcp_quality, HermesEngine};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::thread;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: String) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var(self.key, old);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn quality_review_returns_current_run_findings_json_shape() {
    let _guard = env_lock().lock().expect("env lock");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock gateway");
    let address = format!("http://{}", listener.local_addr().expect("local addr"));
    let response_body = serde_json::json!({
        "model": "hermes-test",
        "choices": [{
            "message": {
                "content": "[{\"tier\":\"T2\",\"line_hint\":2,\"description\":\"Needless collection pass. Complexity: O(N). Verdict: likely avoidable\",\"evidence\":\"numbers.iter().collect::<Vec<_>>()\"}]"
            }
        }],
        "usage": { "prompt_tokens": 50, "completion_tokens": 30, "total_tokens": 80 }
    })
    .to_string();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .expect("set timeout");
        let mut buffer = [0_u8; 4096];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let _gateway = EnvVarGuard::set("HERMES_LLM_GATEWAY_URL", address);
    std::env::remove_var("LLM_GATEWAY_URL");

    let engine = HermesEngine::in_memory("quality-review-json-shape").expect("engine");
    let project_root = tempfile::tempdir().expect("tempdir");
    let file_path = project_root.path().join("sample.rs");
    std::fs::write(
        &file_path,
        "fn sample(numbers: Vec<i32>) -> usize {\n    numbers.iter().collect::<Vec<_>>().len()\n}\n",
    )
    .expect("write sample file");

    let output = mcp_quality::tool_quality_review(
        &engine,
        project_root.path(),
        &json!({
            "path": file_path.to_string_lossy(),
            "dim": "QD-15",
        }),
    )
    .expect("quality review output");

    server.join().expect("server join");

    let payload: serde_json::Value = serde_json::from_str(&output).expect("valid json");
    assert_eq!(payload["files_scanned"], 1);
    assert_eq!(payload["findings_detected"], 1);
    assert_eq!(payload["findings_added"], 1);
    assert!(payload["findings"].is_array());
    assert_eq!(payload["findings"][0]["dim"], "QD-15");
    assert_eq!(payload["findings"][0]["line_hint"], 2);
    assert_eq!(
        payload["findings"][0]["evidence"],
        "numbers.iter().collect::<Vec<_>>()"
    );
}
