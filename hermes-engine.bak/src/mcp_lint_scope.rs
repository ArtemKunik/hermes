// tools/hermes-engine/src/mcp_lint_scope.rs
// Auto-scope resolution for hermes_lint_architecture.
//
// When the caller does not pass an explicit `scope`, derive one from
// git so single-bug fixes do not scan the entire repo.

use std::path::Path;
use std::process::Command;

/// Resolved scope for a lint call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedScope {
    /// "explicit" | "git" | "all"
    pub source: &'static str,
    /// Forward-slash file paths (relative to project_root) used as
    /// substring filters by the lint rule loop.
    pub files: Vec<String>,
}

impl ResolvedScope {
    pub fn explicit(scope: &str) -> Self {
        Self {
            source: "explicit",
            files: vec![scope.replace('\\', "/")],
        }
    }

    pub fn all() -> Self {
        Self {
            source: "all",
            files: Vec::new(),
        }
    }

    pub fn from_git(files: Vec<String>) -> Self {
        Self {
            source: "git",
            files,
        }
    }

    /// Returns true when a violation's file path matches this scope.
    /// `all` matches everything; otherwise we substring-match like the
    /// legacy `scope.contains` behavior.
    pub fn matches(&self, file_path: &str) -> bool {
        if self.files.is_empty() {
            return true;
        }
        let norm = file_path.replace('\\', "/");
        self.files.iter().any(|f| norm.contains(f))
    }
}

/// Run git to enumerate files changed vs HEAD plus untracked files.
/// Returns an empty vec on a clean tree or any git failure.
pub(crate) fn git_changed_files(project_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    // Tracked changes (working tree + staged) vs HEAD.
    if let Some(lines) = run_git(project_root, &["diff", "--name-only", "HEAD"]) {
        out.extend(lines);
    }
    // Untracked but not ignored.
    if let Some(lines) = run_git(
        project_root,
        &["ls-files", "--others", "--exclude-standard"],
    ) {
        out.extend(lines);
    }
    // Dedup while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn run_git(project_root: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(
        text.lines()
            .map(|s| s.trim().replace('\\', "/"))
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_scope_matches_substring() {
        let s = ResolvedScope::explicit("src/foo.rs");
        assert!(s.matches("D:/repo/src/foo.rs"));
        assert!(!s.matches("D:/repo/src/bar.rs"));
        assert_eq!(s.source, "explicit");
    }

    #[test]
    fn all_scope_matches_everything() {
        let s = ResolvedScope::all();
        assert!(s.matches("anything/at/all.rs"));
        assert_eq!(s.source, "all");
    }

    #[test]
    fn git_scope_matches_listed_files_only() {
        let s = ResolvedScope::from_git(vec!["src/a.rs".into(), "src/b.rs".into()]);
        assert!(s.matches("D:/repo/src/a.rs"));
        assert!(s.matches("D:/repo/src/b.rs"));
        assert!(!s.matches("D:/repo/src/c.rs"));
        assert_eq!(s.source, "git");
    }

    #[test]
    fn git_normalizes_backslashes() {
        let s = ResolvedScope::explicit("src\\foo.rs");
        assert!(s.matches("D:/repo/src/foo.rs"));
    }
}
