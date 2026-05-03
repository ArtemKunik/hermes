// tools/hermes-engine/src/quality/zone.rs
// TRACK-049: File zone classification for quality score weighting.

use std::path::Path;

/// Zone determines the score multiplier for findings in that region of the codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Zone {
    /// Core application code: src/, handlers/, *_service/, components/, hooks/
    Production,
    /// Test files: tests/, *_test.rs, *.spec.ts, *.test.ts
    Test,
    /// Config files: *.toml, *.json, *.yaml at root or in infra/
    Config,
    /// Scripts: scripts/, *.ps1, *.sh, *.bat
    Script,
    /// Tooling: tools/, agentd/, node_modules/, target/ — excluded from scoring
    Tooling,
}

impl Zone {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Test => "test",
            Self::Config => "config",
            Self::Script => "script",
            Self::Tooling => "tooling",
        }
    }

    /// Score multiplier per spec: production=1.0, test=0.25, config/script=0.1, tooling=0.0.
    pub fn score_multiplier(&self) -> f64 {
        match self {
            Self::Production => 1.0,
            Self::Test => 0.25,
            Self::Config => 0.1,
            Self::Script => 0.1,
            Self::Tooling => 0.0,
        }
    }

    /// Tooling zone files are excluded entirely from review.
    pub fn is_excluded(&self) -> bool {
        matches!(self, Self::Tooling)
    }
}

/// Classify a file path into a quality Zone.
///
/// Rules evaluated in priority order:
/// 1. Tooling dirs (excluded)
/// 2. Test files (by path segment or extension)
/// 3. Config files (by extension at root/infra)
/// 4. Script files (by path segment or extension)
/// 5. Everything else → Production
pub fn classify_zone(path: &Path) -> Zone {
    let norm = path.to_string_lossy().replace('\\', "/");

    // 1. Tooling — excluded entirely
    let is_tooling = |seg: &str| norm.starts_with(&format!("{seg}/")) || norm.contains(&format!("/{seg}/"));
    if is_tooling("tools")
        || is_tooling("agentd")
        || is_tooling("node_modules")
        || is_tooling("target")
        || is_tooling(".venv")
    {
        return Zone::Tooling;
    }

    let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

    // 2. Test files
    if norm.contains("/tests/")
        || fname.ends_with("_test.rs")
        || fname.ends_with(".spec.ts")
        || fname.ends_with(".test.ts")
        || fname.ends_with(".spec.tsx")
        || fname.ends_with(".test.tsx")
    {
        return Zone::Test;
    }

    // 3. Config files
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(ext, "toml" | "json" | "yaml" | "yml")
        && (norm.contains("/infra/") || !norm.contains("/src/"))
    {
        return Zone::Config;
    }

    // 4. Script files
    if matches!(ext, "ps1" | "sh" | "bat") || norm.contains("/scripts/") {
        return Zone::Script;
    }

    Zone::Production
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn zone(s: &str) -> Zone {
        classify_zone(&PathBuf::from(s))
    }

    #[test]
    fn test_classify_zone_production() {
        assert_eq!(zone("ChartApp/chartapp-server-rust/src/handlers/task.rs"), Zone::Production);
        assert_eq!(zone("ChartApp/chartapp.client/src/components/Chart.tsx"), Zone::Production);
    }

    #[test]
    fn test_classify_zone_test() {
        assert_eq!(zone("ChartApp/chartapp-server-rust/src/task_test.rs"), Zone::Test);
        assert_eq!(zone("ChartApp/chartapp.client/src/Chart.spec.ts"), Zone::Test);
        assert_eq!(zone("ChartApp/chartapp-server-rust/tests/integration.rs"), Zone::Test);
    }

    #[test]
    fn test_classify_zone_config() {
        assert_eq!(zone("infra/main.tf"), Zone::Production); // .tf not in config list
        assert_eq!(zone("Cargo.toml"), Zone::Config);
    }

    #[test]
    fn test_classify_zone_tooling() {
        assert_eq!(zone("tools/hermes-engine/src/main.rs"), Zone::Tooling);
        assert_eq!(zone("agentd/worker.py"), Zone::Tooling);
    }

    #[test]
    fn test_classify_zone_script() {
        assert_eq!(zone("scripts/install-hooks.ps1"), Zone::Script);
        assert_eq!(zone("deploy.sh"), Zone::Script);
    }
}
