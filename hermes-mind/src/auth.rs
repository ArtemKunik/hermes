// One-time OAuth2 consent flow for Gmail and OneDrive.
//
// Usage:
//   hermes-mind auth gmail    --client-id <id> --client-secret <secret>
//   hermes-mind auth onedrive --client-id <id> --client-secret <secret>
//
// Opens a browser to the consent page, starts a local server on :8080 to
// catch the redirect, exchanges the code for tokens, and prints the refresh
// token so you can set the matching HERMES_MIND_*_REFRESH_TOKEN env var.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::TcpListener;

const REDIRECT_URI: &str = "http://localhost:8080/callback";

// ── Gmail ─────────────────────────────────────────────────────────────────────

pub fn auth_gmail(client_id: &str, client_secret: &str) -> Result<()> {
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
         ?client_id={client_id}\
         &redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback\
         &response_type=code\
         &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly\
         &access_type=offline\
         &prompt=consent"
    );

    open_browser(&url);
    eprintln!("[hermes-mind] Opening browser for Gmail consent...");
    eprintln!("[hermes-mind] If browser did not open, visit:\n  {url}");

    let code = wait_for_code()?;
    let token = exchange_code(
        "https://oauth2.googleapis.com/token",
        client_id,
        client_secret,
        &code,
    )?;

    println!("\nGmail authenticated! Set this env var:\n");
    println!("  export HERMES_MIND_GMAIL_CLIENT_ID={client_id}");
    println!("  export HERMES_MIND_GMAIL_CLIENT_SECRET={client_secret}");
    println!("  export HERMES_MIND_GMAIL_REFRESH_TOKEN={token}");
    Ok(())
}

// ── OneDrive ──────────────────────────────────────────────────────────────────

pub fn auth_onedrive(client_id: &str, client_secret: &str) -> Result<()> {
    let url = format!(
        "https://login.microsoftonline.com/common/oauth2/v2.0/authorize\
         ?client_id={client_id}\
         &redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback\
         &response_type=code\
         &scope=Files.Read%20offline_access\
         &prompt=consent"
    );

    open_browser(&url);
    eprintln!("[hermes-mind] Opening browser for OneDrive consent...");
    eprintln!("[hermes-mind] If browser did not open, visit:\n  {url}");

    let code = wait_for_code()?;
    let token = exchange_code(
        "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        client_id,
        client_secret,
        &code,
    )?;

    println!("\nOneDrive authenticated! Set these env vars:\n");
    println!("  export HERMES_MIND_ONEDRIVE_CLIENT_ID={client_id}");
    println!("  export HERMES_MIND_ONEDRIVE_CLIENT_SECRET={client_secret}");
    println!("  export HERMES_MIND_ONEDRIVE_REFRESH_TOKEN={token}");
    Ok(())
}

// ── shared OAuth helpers ──────────────────────────────────────────────────────

/// Spins up a one-shot HTTP listener on :8080 and returns the `code` query param.
fn wait_for_code() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:8080")
        .context("could not bind :8080 — is another process using it?")?;
    eprintln!("[hermes-mind] Waiting for OAuth callback on {REDIRECT_URI} ...");

    let (mut stream, _) = listener.accept()?;

    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // First line: GET /callback?code=<code>&scope=... HTTP/1.1
    let code = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.split('?').nth(1))
        .and_then(|query| {
            query
                .split('&')
                .find(|p| p.starts_with("code="))
                .map(|p| p[5..].to_string())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("no 'code' param in callback — check the redirect URI matches exactly")
        })?;

    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
          <html><body><h2>Authenticated! You can close this tab.</h2></body></html>",
    )?;

    Ok(code)
}

#[derive(Deserialize)]
struct TokenResponse {
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn exchange_code(token_url: &str, client_id: &str, client_secret: &str, code: &str) -> Result<String> {
    let resp: TokenResponse = ureq::post(token_url)
        .send_form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ])
        .context("token exchange request failed")?
        .into_json()
        .context("parsing token response")?;

    if let (Some(e), Some(desc)) = (resp.error, resp.error_description) {
        anyhow::bail!("token exchange error: {e} — {desc}");
    }

    resp.refresh_token
        .ok_or_else(|| anyhow::anyhow!("no refresh_token in response (was access_type=offline set?)"))
}

fn open_browser(url: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    let _ = result; // failure is fine — user sees the URL on stderr
}
