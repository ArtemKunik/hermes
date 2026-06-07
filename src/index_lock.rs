use anyhow::Result;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static LOCK_ACQUISITIONS: AtomicUsize = AtomicUsize::new(0);
static LOCK_CONTENTIONS: AtomicUsize = AtomicUsize::new(0);
static LOCK_RECLAIMS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct LockDetails {
    pub owner_pid: Option<u32>,
    pub acquired_at_unix: u64,
    pub age_secs: u64,
}

pub enum LockAcquisition {
    Acquired(IndexLockGuard),
    Busy(LockDetails),
}

pub struct IndexLockGuard {
    _file: std::fs::File,
    path: PathBuf,
}

impl Drop for IndexLockGuard {
    fn drop(&mut self) {
        log_lock_event("released", &self.path, None);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Generate a unique lock file path in /tmp based on the project root.
/// This avoids polluting project directories and leverages OS cleanup.
pub fn lock_file_path(project_root: &Path) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(project_root.to_string_lossy().as_bytes());
    let hash = hex::encode(hasher.finalize());
    let lock_name = format!("hermes-index-{}.lock", &hash[..16]);
    PathBuf::from("/tmp").join(lock_name)
}

/// Try to acquire an exclusive index lock using OS-level file locking.
///
/// Uses `fs2::FileExt::try_lock_exclusive()` for atomic, crash-safe locking.
/// The OS automatically releases the lock if the process dies — no stale locks.
///
/// Lock files are stored in `/tmp` with a hash of the project root path,
/// avoiding pollution of project directories.
pub fn try_acquire_index_lock(project_root: &Path) -> Result<LockAcquisition> {
    let lock_path = lock_file_path(project_root);

    // Ensure /tmp directory exists
    if let Err(e) = std::fs::create_dir_all(lock_path.parent().unwrap()) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(e.into());
        }
    }

    // Open or create the lock file
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;

    // Try non-blocking exclusive lock
    match file.try_lock_exclusive() {
        Ok(()) => {
            // Write PID and timestamp for debugging
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = file.set_len(0);
            let _ = writeln!(file, "pid={}\nacquired_at={}", std::process::id(), now);

            log_lock_event("acquired", &lock_path, None);
            LOCK_ACQUISITIONS.fetch_add(1, Ordering::Relaxed);
            Ok(LockAcquisition::Acquired(IndexLockGuard {
                _file: file,
                path: lock_path,
            }))
        }
        Err(_) => {
            // Lock is held by another process
            LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);

            // Read lock file for debugging info
            let details = read_lock_details(&lock_path)?;
            log_lock_event("busy", &lock_path, Some(&details));

            Ok(LockAcquisition::Busy(details))
        }
    }
}

fn read_lock_details(lock_path: &Path) -> Result<LockDetails> {
    let content = std::fs::read_to_string(lock_path).unwrap_or_default();
    let pid = content
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<u32>().ok());
    let acquired_at = content
        .lines()
        .find_map(|line| {
            line.strip_prefix("acquired_at=")?
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0);

    let age_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(acquired_at);

    Ok(LockDetails {
        owner_pid: pid,
        acquired_at_unix: acquired_at,
        age_secs,
    })
}

fn log_lock_event(event: &str, lock_path: &Path, details: Option<&LockDetails>) {
    let owner_pid = details
        .and_then(|d| d.owner_pid)
        .map(|p| p.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let age_secs = details.map(|d| d.age_secs).unwrap_or(0);

    eprintln!(
        "[hermes:index-lock] event={event} path={} current_pid={} owner_pid={owner_pid} age_secs={age_secs}",
        lock_path.display(),
        std::process::id(),
    );
}

/// Report lock metrics for debugging contention issues
pub fn report_lock_metrics() {
    eprintln!(
        "[hermes:index-lock] metrics acquisitions={} contentions={} reclaims={}",
        LOCK_ACQUISITIONS.load(Ordering::Relaxed),
        LOCK_CONTENTIONS.load(Ordering::Relaxed),
        LOCK_RECLAIMS.load(Ordering::Relaxed),
    );
}

/// Check if a process with the given PID is currently alive.
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
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn lock_is_exclusive() {
        let temp = tempfile::tempdir().unwrap();
        let acq1 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq1, LockAcquisition::Acquired(_)));

        let acq2 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq2, LockAcquisition::Busy(_)));

        drop(acq1);

        // Give OS time to release the lock
        thread::sleep(Duration::from_millis(100));

        let acq3 = try_acquire_index_lock(temp.path()).unwrap();
        assert!(matches!(acq3, LockAcquisition::Acquired(_)));
    }

    #[test]
    fn lock_file_in_tmp_with_hash() {
        let path = Path::new("/home/user/my-project");
        let lock_path = lock_file_path(path);

        assert!(lock_path.starts_with("/tmp"));
        assert!(lock_path.to_string_lossy().contains("hermes-index-"));
        assert!(lock_path.to_string_lossy().ends_with(".lock"));
    }

    #[test]
    fn same_project_same_lock_different_project_different_lock() {
        let path_a = Path::new("/home/user/project-a");
        let path_b = Path::new("/home/user/project-b");
        let path_a2 = Path::new("/home/user/project-a");

        let lock_a = lock_file_path(path_a);
        let lock_b = lock_file_path(path_b);
        let lock_a2 = lock_file_path(path_a2);

        assert_eq!(lock_a, lock_a2);
        assert_ne!(lock_a, lock_b);
    }
}
