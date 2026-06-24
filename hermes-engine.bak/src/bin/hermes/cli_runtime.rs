use anyhow::Result;
use hermes_engine::HermesEngine;
use std::{
    env,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub fn find_git_root(start: &Path) -> PathBuf {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return cur;
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
}

pub fn open_engine(command: &str) -> Result<(HermesEngine, PathBuf)> {
    let project_root = env::var("HERMES_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            find_git_root(&cwd)
        });

    let db_path = env::var("HERMES_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_root.join(".hermes.db"));

    let project_id = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let retries = if command == "index" {
        0
    } else {
        hermes_engine::retry::lock_retry_budget()
    };
    let mut attempt = 0usize;

    loop {
        match HermesEngine::new(&db_path, &project_id) {
            Ok(engine) => return Ok((engine, project_root.clone())),
            Err(err)
                if attempt < retries
                    && hermes_engine::retry::is_database_locked_message(&err.to_string()) =>
            {
                let delay_ms = hermes_engine::retry::lock_retry_delay_ms(attempt);
                eprintln!(
                    "[hermes] open_engine lock retry attempt={}/{} delay={}ms",
                    attempt + 1,
                    retries,
                    delay_ms
                );
                thread::sleep(Duration::from_millis(delay_ms));
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}
