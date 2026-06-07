use anyhow::Result;
use std::path::Path;

const HERMES_HOOK_HEADER: &str = "# HERMES HOOK v1 — managed by `hermes install-hook`";

pub fn generate_hook_script(threshold: f64, strict: bool) -> String {
    let strict_line = if strict {
        "STRICT=true"
    } else {
        "STRICT=false"
    };

    format!(
        r#"#!/bin/sh
{header}

THRESHOLD={threshold}
{strict_line}

HERMES_DB="$(git rev-parse --show-toplevel)/.hermes.db"

if [ ! -f "$HERMES_DB" ]; then
    exit 0
fi

PROJECT_ID="$(basename "$(git rev-parse --show-toplevel)")"

for file in $(git diff --cached --name-only); do
    score=$(sqlite3 "$HERMES_DB" "SELECT bs.blast_score FROM blast_scores bs WHERE bs.project_id = '$PROJECT_ID' AND bs.file_path = '$file' AND bs.blast_score > $threshold" 2>/dev/null)
    if [ -n "$score" ]; then
        echo "[hermes] high blast-radius file staged: $file (score: $score)"
        if [ "$STRICT" = true ]; then
            exit 1
        fi
    fi
done
"#,
        header = HERMES_HOOK_HEADER,
        threshold = threshold,
        strict_line = strict_line,
    )
}

pub fn install_hook(git_root: &Path, script: &str) -> Result<()> {
    let hooks_dir = git_root.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    let hook_path = hooks_dir.join("pre-commit");
    std::fs::write(&hook_path, script)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    Ok(())
}

pub fn remove_hook(git_root: &Path) -> Result<bool> {
    let hook_path = git_root.join(".git").join("hooks").join("pre-commit");
    if !hook_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&hook_path)?;
    if content.contains(HERMES_HOOK_HEADER) {
        std::fs::remove_file(&hook_path)?;
        Ok(true)
    } else {
        anyhow::bail!("pre-commit hook exists but is not managed by hermes — refusing to remove");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_generate_hook_script_default() {
        let script = generate_hook_script(10.0, false);
        assert!(script.contains("THRESHOLD=10"));
        assert!(script.contains("STRICT=false"));
        assert!(script.contains("HERMES HOOK v1"));
        assert!(script.contains("sqlite3"));
        assert!(script.starts_with("#!/bin/sh"));
    }

    #[test]
    fn test_generate_hook_script_strict() {
        let script = generate_hook_script(5.0, true);
        assert!(script.contains("THRESHOLD=5"));
        assert!(script.contains("STRICT=true"));
    }

    #[test]
    fn test_install_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir_all(git_dir.join("hooks")).unwrap();

        let script = generate_hook_script(10.0, false);
        install_hook(dir.path(), &script).unwrap();

        let hook_path = dir.path().join(".git").join("hooks").join("pre-commit");
        assert!(hook_path.exists());

        let installed = std::fs::read_to_string(&hook_path).unwrap();
        assert!(installed.contains("HERMES HOOK v1"));

        let removed = remove_hook(dir.path()).unwrap();
        assert!(removed);
        assert!(!hook_path.exists());
    }

    #[test]
    fn test_remove_refuses_non_hermes_hook() {
        let dir = tempfile::tempdir().unwrap();
        let hook_path = dir.path().join(".git").join("hooks").join("pre-commit");
        std::fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
        std::fs::write(&hook_path, "#!/bin/sh\necho custom hook\n").unwrap();

        let result = remove_hook(dir.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not managed by hermes"));
    }

    #[test]
    fn test_remove_nonexistent_hook() {
        let dir = tempfile::tempdir().unwrap();
        let result = remove_hook(dir.path()).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_generated_script_setuid() {
        let script = generate_hook_script(10.0, true);
        assert!(script.contains("exit 1"));
    }
}
