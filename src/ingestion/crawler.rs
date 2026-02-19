use anyhow::Result;
use std::path::{Path, PathBuf};

const SUPPORTED_EXTENSIONS: &[&str] =
    &["rs", "tsx", "ts", "jsx", "js", "md", "toml", "json", "css"];

const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "dist",
    ".next",
    ".vite",
];

pub fn crawl_directory(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    crawl_recursive(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn crawl_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if IGNORED_DIRS.contains(&dir_name.as_str()) {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            crawl_recursive(&path, files)?;
        } else if is_supported_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn crawl_finds_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("readme.txt"), "not indexed").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn crawl_ignores_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("lib.js"), "module.exports = {}").unwrap();
        fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("app.ts"));
    }

    #[test]
    fn supported_extensions_check() {
        assert!(is_supported_file(Path::new("foo.rs")));
        assert!(is_supported_file(Path::new("bar.tsx")));
        assert!(is_supported_file(Path::new("doc.md")));
        assert!(!is_supported_file(Path::new("image.png")));
        assert!(!is_supported_file(Path::new("data.csv")));
    }
}
