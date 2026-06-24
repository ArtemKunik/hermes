use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

// These integration tests compile the Hermes binary and invoke it with CLI args.
// They exercise the new "validate-env" and "validate-symbols" commands added in
// the previous commit.

#[test]
fn cli_validate_env_known() {
    // the in-memory database used during tests starts empty, so validate-env on any
    // name should return valid:false (no registry entries yet) without crashing.
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("validate-env").arg("FOO_BAR");
    cmd.assert().success().stdout(contains("\"valid\": false"));
}

#[test]
fn cli_validate_env_usage_error() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("validate-env");
    cmd.assert().failure().stderr(contains("usage"));
}

#[test]
fn cli_validate_symbols_usage() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("validate-symbols");
    cmd.assert().failure().stderr(contains("usage"));
}

#[test]
fn cli_validate_symbols_unknown() {
    // run against a fresh in-memory engine, so no symbols exist; result should
    // mark the given symbol as invalid.
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("validate-symbols").arg("does_not_exist");
    cmd.assert()
        .success()
        .stdout(contains("\"valid\": false"))
        .stdout(contains("does_not_exist"));
}

#[test]
fn cli_scan_duplicates_usage() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("scan-duplicates");
    cmd.assert().failure().stderr(contains("usage"));
}

#[test]
fn cli_scan_duplicates_basic() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("scan-duplicates").arg("some_signature");
    cmd.assert()
        .success()
        .stdout(contains("\"has_duplicates\""));
}

#[test]
fn cli_prepare_commit_message_usage_error() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("prepare-commit-message");
    cmd.assert().failure().stderr(contains("usage"));
}

#[test]
fn cli_prepare_commit_message_infers_pipeline_from_changes() {
    let mut cmd = cargo_bin_cmd!("Hermes");
    cmd.arg("prepare-commit-message")
        .arg("fix(ccterm): keep SRE build context")
        .arg("--task")
        .arg("task://build-heal/2026-03-10-001")
        .arg("--decision")
        .arg("memory/decisions/build-heal-context-linking.md")
        .arg("--docs")
        .arg("docs/sre-dashboard.md")
        .arg("--changes")
        .arg("tools/ccterm/src/web_ui/app_notifications.js");

    cmd.assert()
        .success()
        .stdout(contains("Task-Model: task://build-heal/2026-03-10-001"))
        .stdout(contains("Decision-Doc: memory/decisions/build-heal-context-linking.md"))
        .stdout(contains("Docs: docs/sre-dashboard.md"))
        .stdout(contains("Pipeline: 18"));
}
