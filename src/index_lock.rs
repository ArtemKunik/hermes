use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STALE_LOCK_AGE: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
pub struct LockDetails {
    pub owner_pid: Option<u32>,
    pub acquired_at_unix: u64,
    pub age: Duration,
    pub is_stale: bool,
}

pub enum LockAcquisition {
    Acquired(IndexLockGuard),
    Busy(LockDetails),
}

#[derive(Debug)]
struct LockFileSnapshot {
    age: Duration,
    pid: Option<u32>,
    pid_is_alive: Option<bool>,
    acquired_at: u64,
    should_reclaim: bool,
}

pub struct IndexLockGuard {
    path: PathBuf,
}

impl Drop for IndexLockGuard {
    fn drop(&mut self) {
        log_lock_event("released", &self.path, None);
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn try_acquire_index_lock(project_root: &Path) -> Result<LockAcquisition> {
    let lock_path = project_root.join(".hermes.index.lock");
    match try_create_lock(&lock_path) {
        Ok(lock) => {
            log_lock_event("acquired", &lock_path, None);
            Ok(LockAcquisition::Acquired(lock))
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let snapshot = inspect_lock_file(&lock_path)?;
            if snapshot.should_reclaim {
                log_lock_event("reclaiming-stale", &lock_path, Some(&snapshot));
                let _ = std::fs::remove_file(&lock_path);
                let lock = try_create_lock(&lock_path)?;
                log_lock_event("reacquired", &lock_path, Some(&snapshot));
                return Ok(LockAcquisition::Acquired(lock));
            }
            log_lock_event("busy", &lock_path, Some(&snapshot));
            Ok(LockAcquisition::Busy(LockDetails {
                owner_pid: snapshot.pid,
                acquired_at_unix: snapshot.acquired_at,
                age: snapshot.age,
                is_stale: snapshot.age >= STALE_LOCK_AGE,
            }))
        }
        Err(err) => {
            eprintln!(
                "[hermes:index-lock] event=create-failed path={} current_pid={} error={err}",
                lock_path.display(),
                std::process::id()
            );
            Err(err.into())
        }
    }
}

fn inspect_lock_file(lock_path: &Path) -> Result<LockFileSnapshot> {
    let content = std::fs::read_to_string(lock_path).unwrap_or_default();
    let pid = content.lines().find_map(|line| line.strip_prefix("pid=")?.trim().parse::<u32>().ok());
    let acquired_at = content.lines().find_map(|line| line.strip_prefix("acquired_at=")?.trim().parse::<u64>().ok()).unwrap_or(0);
    
    let metadata = std::fs::metadata(lock_path)?;
    let age = metadata.modified()?.elapsed().unwrap_or_default();
    let pid_is_alive = pid.map(process_is_alive);
    
    Ok(LockFileSnapshot {
        age,
        pid,
        pid_is_alive,
        acquired_at,
        should_reclaim: age >= STALE_LOCK_AGE || pid_is_alive == Some(false),
    })
}

fn log_lock_event(event: &str, lock_path: &Path, snapshot: Option<&LockFileSnapshot>) {
    let owner_pid = snapshot
        .and_then(|value| value.pid)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let owner_alive = snapshot
        .and_then(|value| value.pid_is_alive)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let age_ms = snapshot
        .map(|value| value.age.as_millis().to_string())
        .unwrap_or_else(|| "0".to_string());
    eprintln!(
        "[hermes:index-lock] event={event} path={} current_pid={} owner_pid={owner_pid} owner_alive={owner_alive} age_ms={age_ms}",
        lock_path.display(),
        std::process::id()
    );
}

fn try_create_lock(lock_path: &Path) -> std::io::Result<IndexLockGuard> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = writeln!(file, "pid={}\nacquired_at={}", std::process::id(), now);
    Ok(IndexLockGuard {
        path: lock_path.to_path_buf(),
    })
}

pub fn process_is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        return output
            .ok()
            .filter(|value| value.status.success())
            .map(|value| String::from_utf8_lossy(&value.stdout).contains(&pid.to_string()))
            .unwrap_or(false);
    }

    #[cfg(not(windows))]
    {
        let std_proc = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        std_proc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lock_snapshot_marks_dead_pid_as_reclaimable() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join(".hermes.index.lock");
        fs::write(&lock_path, "pid=999999").unwrap();

        let snapshot = inspect_lock_file(&lock_path).unwrap();

        assert_eq!(snapshot.pid, Some(999999));
        assert_eq!(snapshot.pid_is_alive, Some(false));
        assert!(snapshot.should_reclaim);
    }

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let acq1 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq1, LockAcquisition::Acquired(_)));

        let acq2 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq2, LockAcquisition::Busy(_)));

        drop(acq1);
        let acq3 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq3, LockAcquisition::Acquired(_)));
    }

    #[test]
    fn stale_dead_pid_lock_is_reclaimed() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join(".hermes.index.lock");
        fs::write(&lock_path, "pid=999999").unwrap();

        let acq = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq, LockAcquisition::Acquired(_)));
    }
}
