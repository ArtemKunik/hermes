use anyhow::Result;
use regex::Regex;
use rusqlite::Connection;
use std::path::Path;

/// Well-known system / CI framework / third-party variables that are never
/// project-specific and should not be flagged by the guard.
pub const REGISTRY_WHITELIST: &[&str] = &[
    "CI",
    "TF_BUILD",
    "BUILD_BUILDID",
    "BUILD_SOURCEBRANCH",
    "PLAYWRIGHT_BASE_URL",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMFILES",
    "COMPUTERNAME",
    "USERNAME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "PORT",
    "HOST",
    "KV_REST_API_URL",
    "KV_REST_API_TOKEN",
    "UPSTASH_REDIS_REST_URL",
    "UPSTASH_REDIS_REST_TOKEN",
];

/// Environment variable scanner for populating config_registry.
pub struct EnvScanner {
    usage_patterns: Vec<EnvPattern>,
    definition_patterns: Vec<EnvPattern>,
    js_destructure_re: Regex,
}

struct EnvPattern {
    regex: Regex,
    language: String,
    var_capture_index: usize,
}

impl EnvScanner {
    pub fn new() -> Result<Self> {
        let usage_patterns = vec![
            EnvPattern {
                regex: Regex::new(
                    r##"(?:os\.)?(?:getenv|environ(?:\[\s*['\"]|(?:\.get\(|\[\s*['\"])))([^'\"\]\)]+)['\"]"##,
                )?,
                language: "python".to_string(),
                var_capture_index: 1,
            },
            EnvPattern {
                regex: Regex::new(
                    r##"process\.env(?:\.([A-Z_][A-Z0-9_]*)|\[\s*['\"]([^'\"]+)['\"]\s*\])"##,
                )?,
                language: "javascript".to_string(),
                var_capture_index: 1,
            },
            EnvPattern {
                regex: Regex::new(r##"(?:std::)?env::var\(['\"]([^'\"]+)['\"]\)"##)?,
                language: "rust".to_string(),
                var_capture_index: 1,
            },
            EnvPattern {
                regex: Regex::new(r"\$\{?([A-Z_][A-Z0-9_]*)\}?")?,
                language: "shell".to_string(),
                var_capture_index: 1,
            },
        ];

        let definition_patterns = vec![
            EnvPattern {
                regex: Regex::new(r"(?m)^\s*([A-Z_][A-Z0-9_]*)\s*=")?,
                language: "env".to_string(),
                var_capture_index: 1,
            },
            EnvPattern {
                regex: Regex::new(r"(?m)^\s*-\s*([A-Z_][A-Z0-9_]*)(?:\s*[:=]|\s*$)")?,
                language: "yaml".to_string(),
                var_capture_index: 1,
            },
            // Markdown tables (e.g. | `VAR_NAME` |)
            EnvPattern {
                regex: Regex::new(r"\|\s*`?([A-Z_][A-Z0-9_]*)`?\s*\|")?,
                language: "markdown".to_string(),
                var_capture_index: 1,
            },
        ];

        let js_destructure_re = Regex::new(r"\{([^}]*)\}\s*=\s*process\.env\b")?;

        Ok(Self {
            usage_patterns,
            definition_patterns,
            js_destructure_re,
        })
    }

    pub fn scan_file(&self, file_path: &Path, content: &str) -> Vec<DiscoveredEnvVar> {
        let mut vars = Vec::new();
        let file_ext = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let file_name = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        // Check definition patterns for definition-style files
        if file_name.starts_with(".env")
            || file_ext == "yaml"
            || file_ext == "yml"
            || file_ext == "md"
        {
            for pattern in &self.definition_patterns {
                if !self.pattern_matches_file(pattern, file_ext, file_name) {
                    continue;
                }
                for cap in pattern.regex.captures_iter(content) {
                    if let Some(m) = cap.get(pattern.var_capture_index) {
                        let name = m.as_str().to_string();
                        if REGISTRY_WHITELIST.contains(&name.as_str()) {
                            continue;
                        }
                        vars.push(DiscoveredEnvVar {
                            name,
                            file_path: file_path.to_string_lossy().to_string(),
                            is_definition: true,
                        });
                    }
                }
            }
        }

        // Check usage patterns
        for pattern in &self.usage_patterns {
            if !self.pattern_matches_file(pattern, file_ext, file_name) {
                continue;
            }
            for cap in pattern.regex.captures_iter(content) {
                let var_name = if pattern.language == "javascript" {
                    cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str().to_string())
                } else {
                    cap.get(pattern.var_capture_index).map(|m| m.as_str().to_string())
                };
                if let Some(name) = var_name {
                    if REGISTRY_WHITELIST.contains(&name.as_str()) {
                        continue;
                    }
                    vars.push(DiscoveredEnvVar {
                        name,
                        file_path: file_path.to_string_lossy().to_string(),
                        is_definition: false,
                    });
                }
            }
        }

        // Special check for JS/TS destructuring: const { VAR } = process.env
        if matches!(file_ext, "js" | "ts" | "jsx" | "tsx") {
            for cap in self.js_destructure_re.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    for item in m.as_str().split(',') {
                        let key = item.split(':').next().unwrap()
                            .split('=').next().unwrap()
                            .trim();
                        if !key.is_empty()
                            && key.chars().all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
                            && !REGISTRY_WHITELIST.contains(&key)
                        {
                            vars.push(DiscoveredEnvVar {
                                name: key.to_string(),
                                file_path: file_path.to_string_lossy().to_string(),
                                is_definition: false,
                            });
                        }
                    }
                }
            }
        }

        vars
    }

    fn pattern_matches_file(&self, pattern: &EnvPattern, file_ext: &str, file_name: &str) -> bool {
        match pattern.language.as_str() {
            "python"     => file_ext == "py",
            "javascript" => matches!(file_ext, "js" | "ts" | "jsx" | "tsx"),
            "rust"       => file_ext == "rs",
            "shell"      => matches!(file_ext, "sh" | "bash" | "zsh"),
            "env"        => file_name.starts_with(".env"),
            "yaml"       => matches!(file_ext, "yaml" | "yml"),
            "markdown"   => file_ext == "md",
            _            => true,
        }
    }

    pub fn scan_files(&self, files: &[(String, String)]) -> Vec<DiscoveredEnvVar> {
        let mut all = Vec::new();
        for (path, content) in files {
            all.extend(self.scan_file(Path::new(path), content));
        }
        all
    }

    /// Populate config_registry with discovered vars.
    ///
    /// Each key gets one row with two boolean flags — `is_defined` (seen in a .env/yaml/md
    /// definition context) and `is_used` (seen accessed in code).  Either flag flips to 1 as
    /// new occurrences are encountered; it never resets to 0.
    pub fn populate_registry(
        &self,
        conn: &Connection,
        _project_id: &str,
        discovered_vars: &[DiscoveredEnvVar],
    ) -> Result<()> {
        for var in discovered_vars {
            if var.is_definition {
                conn.execute(
                    "INSERT INTO config_registry (key, is_defined, is_used) VALUES (?1, 1, 0)
                     ON CONFLICT(key) DO UPDATE SET is_defined = 1",
                    [&var.name],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO config_registry (key, is_defined, is_used) VALUES (?1, 0, 1)
                     ON CONFLICT(key) DO UPDATE SET is_used = 1",
                    [&var.name],
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredEnvVar {
    pub name: String,
    pub file_path: String,
    pub is_definition: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_rust_env_var_usage_detected() {
        let scanner = EnvScanner::new().unwrap();
        let content = r#"let val = std::env::var("MY_SECRET_KEY").unwrap();"#;
        let vars = scanner.scan_file(Path::new("main.rs"), content);
        assert!(vars.iter().any(|v| v.name == "MY_SECRET_KEY" && !v.is_definition));
    }

    #[test]
    fn test_dotenv_definition_detected() {
        let scanner = EnvScanner::new().unwrap();
        let content = "MY_SECRET_KEY=supersecret\nDB_URL=postgres://localhost/db\n";
        let vars = scanner.scan_file(Path::new(".env"), content);
        assert!(vars.iter().any(|v| v.name == "MY_SECRET_KEY" && v.is_definition));
        assert!(vars.iter().any(|v| v.name == "DB_URL" && v.is_definition));
    }

    #[test]
    fn test_js_destructuring() {
        let scanner = EnvScanner::new().unwrap();
        let content = "const { API_KEY, DB_URL: url } = process.env;";
        let vars = scanner.scan_file(Path::new("test.ts"), content);
        assert!(vars.iter().any(|v| v.name == "API_KEY"));
        assert!(vars.iter().any(|v| v.name == "DB_URL"));
    }

    #[test]
    fn test_whitelist_excluded() {
        let scanner = EnvScanner::new().unwrap();
        let content = "let port = std::env::var(\"PORT\").unwrap_or_default();";
        let vars = scanner.scan_file(Path::new("main.rs"), content);
        assert!(!vars.iter().any(|v| v.name == "PORT"), "PORT is whitelisted and must be excluded");
    }
}
