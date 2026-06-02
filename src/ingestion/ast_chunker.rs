use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AstChunk {
    pub name: String,
    pub content: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub object_type: String,
}

pub struct AstChunker;

#[cfg(feature = "ast")]
impl AstChunker {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn chunk_file(&mut self, file_path: &Path, content: &str) -> Result<Vec<AstChunk>> {
        let ext = file_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let extractor = crate::ingestion::lang::get_extractor(ext)
            .ok_or_else(|| anyhow::anyhow!("unsupported file extension: {ext}"))?;
        Ok(extractor.extract_symbols(content, &file_path.to_string_lossy()))
    }
}

#[cfg(not(feature = "ast"))]
impl AstChunker {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn chunk_file(&mut self, _file_path: &Path, _content: &str) -> Result<Vec<AstChunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "ast")]
    use super::*;

    #[cfg(feature = "ast")]
    #[test]
    fn test_rust_function_chunking() {
        let mut chunker = AstChunker::new().unwrap();
        let content = r#"
fn hello_world() {
    println!("Hello, world!");
}
"#;
        let path = std::path::Path::new("test.rs");
        let chunks = chunker.chunk_file(path, content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello_world");
        assert_eq!(chunks[0].object_type, "function");
        assert!(chunks[0].content.contains("fn hello_world"));
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_typescript_function_chunking() {
        let mut chunker = AstChunker::new().unwrap();
        let content = "function greet(name: string): string { return `hello ${name}`; }";
        let path = std::path::Path::new("test.ts");
        let chunks = chunker.chunk_file(path, content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "greet");
        assert_eq!(chunks[0].object_type, "function");
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_python_function_chunking() {
        let mut chunker = AstChunker::new().unwrap();
        let content = "def hello(name):\n    return f'hello {name}'\n";
        let path = std::path::Path::new("test.py");
        let chunks = chunker.chunk_file(path, content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].object_type, "function");
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_unsupported_extension_returns_error() {
        let mut chunker = AstChunker::new().unwrap();
        let path = std::path::Path::new("test.go");
        let result = chunker.chunk_file(path, "");
        assert!(result.is_err());
    }
}
