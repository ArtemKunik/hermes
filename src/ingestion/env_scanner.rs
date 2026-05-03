// src/ingestion/env_scanner.rs
// TRACK-040 Phase 1: Environment variable scanner for hallucination prevention
//
// Scans source files for environment variable access patterns and populates
// config_registry table with discovered variables for validation.

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

/// Environment variable scanner for populating config_registry
pub struct EnvScanner {
    /// Regex patterns for different languages/frameworks
    usage_patterns: Vec<EnvPattern>,
    /// Regex patterns for definition files (.env, docker-compose, etc)
    definition_patterns: Vec<EnvPattern>,
    /// Pattern for JS destructuring: const { VAR } = process.env
    js_destructure_re: Regex,
}

struct EnvPattern {
    /// Compiled regex for matching env var access
    regex: Regex,
    /// Language or framework this pattern applies to
    language: String,
    /// Capture group index for the variable name (1-based)
    var_capture_index: usize,
}

impl EnvScanner {
    pub fn new() -> Result<Self> {
        let usage_patterns = vec![
            // Python: os.getenv('VAR') or os.environ.get('VAR')
            EnvPattern {
                regex: Regex::new(
                    r##"(?:os\.)?(?:getenv|environ(?:\[\s*['\"]|(?:\.get\(|\[\s*['\"])))([^'\"\]\)]+)['\"]"##,
                )?,
                language: "python".to_string(),
                var_capture_index: 1,
            },
            // JavaScript/Node: process.env.VAR or process.env['VAR']
            EnvPattern {
                regex: Regex::new(
                    r##"process\.env(?:\.([A-Z_][A-Z0-9_]*)|\[\s*['\"]([^'\"]+)['\"]\s*\])"##,
                )?,
                language: "javascript".to_string(),
                var_capture_index: 1,
            },
            // Rust: std::env::var("VAR") or env::var("VAR")
            EnvPattern {
                regex: Regex::new(r##"(?:std::)?env::var\(['\"]([^'\"]+)['\"]\)"##)?,
                language: "rust".to_string(),
                var_capture_index: 1,
            },
            // Shell/Bash: $VAR or ${VAR}
            EnvPattern {
                regex: Regex::new(r"\$\{?([A-Z_][A-Z0-9_]*)\}?")?,
                language: "shell".to_string(),
                var_capture_index: 1,
            },
        ];

        let definition_patterns = vec![
            // .env files: VAR=value
            EnvPattern {
                regex: Regex::new(r"(?m)^\s*([A-Z_][A-Z0-9_]*)\s*=")?,
                language: "env".to_string(),
                var_capture_index: 1,
            },
            // docker-compose: - VAR=value or - VAR
            EnvPattern {
                regex: Regex::new(r"(?m)^\s*-\s*([A-Z_][A-Z0-9_]*)(?:\s*[:=]|\s*$)")?,
                language: "yaml".to_string(),
                var_capture_index: 1,
            },
            // Markdown tables (e.g. ENDPOINTS_AND_ENV_VARS.md): | VAR |
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

    /// Scan a file for environment variable usage and return discovered variables
    pub fn scan_file(&self, file_path: &Path, content: &str) -> Vec<DiscoveredEnvVar> {
        let mut vars = Vec::new();
        let file_ext = file_path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let file_name = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        // Check definition patterns
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
                    if let Some(var_match) = cap.get(pattern.var_capture_index) {
                        let name = var_match.as_str().to_string();
                        if REGISTRY_WHITELIST.contains(&name.as_str()) {
                            continue;
                        }
                        vars.push(DiscoveredEnvVar {
                            name,
                            source: pattern.language.clone(),
                            file_path: file_path.to_string_lossy().to_string(),
                            context: self.extract_context(content, cap.get(0).unwrap().start()),
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
                    cap.get(1)
                        .or_else(|| cap.get(2))
                        .map(|m| m.as_str().to_string())
                } else {
                    cap.get(pattern.var_capture_index)
                        .map(|m| m.as_str().to_string())
                };

                if let Some(name) = var_name {
                    if REGISTRY_WHITELIST.contains(&name.as_str()) {
                        continue;
                    }
                    vars.push(DiscoveredEnvVar {
                        name,
                        source: pattern.language.clone(),
                        file_path: file_path.to_string_lossy().to_string(),
                        context: self.extract_context(content, cap.get(0).unwrap().start()),
                        is_definition: false,
                    });
                }
            }
        }

        // Special check for JS destructuring
        if matches!(file_ext, "js" | "ts" | "jsx" | "tsx") {
            for cap in self.js_destructure_re.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    let items = m.as_str().split(',');
                    for item in items {
                        // Extract key from "key: alias" or "key = default"
                        let key = item
                            .split(':')
                            .next()
                            .unwrap()
                            .split('=')
                            .next()
                            .unwrap()
                            .trim();
                        if !key.is_empty()
                            && key
                                .chars()
                                .all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
                        {
                            if REGISTRY_WHITELIST.contains(&key) {
                                continue;
                            }
                            vars.push(DiscoveredEnvVar {
                                name: key.to_string(),
                                source: "javascript".to_string(),
                                file_path: file_path.to_string_lossy().to_string(),
                                context: self.extract_context(content, cap.get(0).unwrap().start()),
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
            "python" => file_ext == "py",
            "javascript" => matches!(file_ext, "js" | "ts" | "jsx" | "tsx"),
            "rust" => file_ext == "rs",
            "shell" => matches!(file_ext, "sh" | "bash" | "zsh"),
            "env" => file_name.starts_with(".env"),
            "yaml" => matches!(file_ext, "yaml" | "yml"),
            "markdown" => file_ext == "md",
            _ => true,
        }
    }

    fn extract_context(&self, content: &str, pos: usize) -> String {
        let mut start = pos.saturating_sub(50);
        let mut end = (pos + 50).min(content.len());
        while start < pos && !content.is_char_boundary(start) {
            start += 1;
        }
        while end > pos && !content.is_char_boundary(end) {
            end -= 1;
        }
        content[start..end].to_string()
    }

    /// Strips comments while preserving line breaks to maintain line number accuracy.
    pub fn strip_comments(content: &str) -> String {
        // Simple line comment stripping (# and //)
        let mut result = String::with_capacity(content.len());
        for line in content.lines() {
            if let Some(idx) = line.find("//") {
                result.push_str(&line[..idx]);
            } else if let Some(idx) = line.find('#') {
                // Heuristic: Avoid stripping # inside strings if possible,
                // but for ENV usage detection, simple is usually fine.
                result.push_str(&line[..idx]);
            } else {
                result.push_str(line);
            }
            result.push('\n');
        }
        result
    }

    pub fn scan_files(&self, files: &[(String, String)]) -> Vec<DiscoveredEnvVar> {
        let mut all_vars = Vec::new();
        for (file_path, content) in files {
            let path = Path::new(file_path);
            all_vars.extend(self.scan_file(path, content));
        }
        all_vars
    }

    pub fn populate_registry(
        &self,
        conn: &Connection,
        _project_id: &str,
        discovered_vars: &[DiscoveredEnvVar],
    ) -> Result<()> {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO config_registry (key, value, source) VALUES (?, '', ?)",
        )?;

        for var in discovered_vars {
            let source = if var.is_definition { "defined" } else { "used" };
            stmt.execute([&var.name, source])?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredEnvVar {
    pub name: String,
    pub source: String,
    pub file_path: String,
    pub context: String,
    pub is_definition: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extract_context_handles_multibyte_boundaries() {
        let scanner = EnvScanner::new().unwrap();
        let text = "start — middle $VAR end";
        let pos = text.find("$VAR").unwrap();
        let ctx = scanner.extract_context(text, pos);
        assert!(ctx.contains("$VAR"));
    }

    #[test]
    fn test_js_destructuring() {
        let scanner = EnvScanner::new().unwrap();
        let content = "const { API_KEY, DB_URL: url, PORT = 3000 } = process.env;";
        let vars = scanner.scan_file(Path::new("test.ts"), content);
        assert!(vars.iter().any(|v| v.name == "API_KEY"));
        assert!(vars.iter().any(|v| v.name == "DB_URL"));
        // PORT is whitelisted
        assert!(!vars.iter().any(|v| v.name == "PORT"));
    }
}
