// src/ingestion/ast_chunker.rs
// TRACK-040 Phase 2: AST-aware chunking with tree-sitter
//
// Replaces regex-based text chunking with AST parsing for accurate
// code structure extraction. Feature-gated behind "ast" flag.

#[cfg(feature = "ast")]
use anyhow::Result;
#[cfg(feature = "ast")]
use std::path::Path;
#[cfg(feature = "ast")]
use tree_sitter::{Language, Parser, Tree};

#[cfg(feature = "ast")]
pub struct AstChunker {
    parser: Parser,
}

#[cfg(feature = "ast")]
impl AstChunker {
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();

        // For now, support Rust and TypeScript/JavaScript
        // TODO: Add more languages as needed
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language)?;

        Ok(Self { parser })
    }

    pub fn chunk_file(&mut self, file_path: &Path, content: &str) -> Result<Vec<AstChunk>> {
        let tree = self
            .parser
            .parse(content, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse file: {}", file_path.display()))?;

        let mut chunks = Vec::new();
        self.extract_chunks(&tree, content, &mut chunks, file_path);

        Ok(chunks)
    }

    fn extract_chunks(
        &self,
        tree: &Tree,
        content: &str,
        chunks: &mut Vec<AstChunk>,
        file_path: &Path,
    ) {
        let mut cursor = tree.walk();

        // Traverse the AST and extract meaningful nodes
        self.traverse_node(&mut cursor, content, chunks, file_path);
    }

    fn traverse_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        chunks: &mut Vec<AstChunk>,
        file_path: &Path,
    ) {
        let node = cursor.node();

        // Extract chunks for function definitions, struct definitions, etc.
        if self.is_chunkable_node(&node) {
            if let Some(chunk) = self.create_chunk(&node, content, file_path) {
                chunks.push(chunk);
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            loop {
                self.traverse_node(cursor, content, chunks, file_path);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    fn is_chunkable_node(&self, node: &tree_sitter::Node) -> bool {
        matches!(
            node.kind(),
            "function_item" | "struct_item" | "enum_item" | "impl_item" | "trait_item" | "mod_item"
        )
    }

    fn create_chunk(
        &self,
        node: &tree_sitter::Node,
        content: &str,
        file_path: &Path,
    ) -> Option<AstChunk> {
        let range = node.range();
        let chunk_content = &content[range.start_byte..range.end_byte];

        // Extract name from the node
        let name = self.extract_node_name(node, content)?;

        let object_type = match node.kind() {
            "function_item" => "function",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "impl_item" => "impl",
            "trait_item" => "trait",
            "mod_item" => "module",
            _ => "unknown",
        };

        Some(AstChunk {
            name,
            content: chunk_content.to_string(),
            file_path: file_path.to_string_lossy().to_string(),
            start_line: range.start_point.row + 1, // 1-based
            end_line: range.end_point.row + 1,
            object_type: object_type.to_string(),
        })
    }

    fn extract_node_name(&self, node: &tree_sitter::Node, content: &str) -> Option<String> {
        // Find the identifier node within this node
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "identifier" || child.kind() == "type_identifier" {
                    let range = child.range();
                    return Some(content[range.start_byte..range.end_byte].to_string());
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }
}

#[cfg(feature = "ast")]
#[derive(Debug, Clone)]
pub struct AstChunk {
    pub name: String,
    pub content: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub object_type: String,
}

#[cfg(not(feature = "ast"))]
pub struct AstChunker;

#[cfg(not(feature = "ast"))]
impl AstChunker {
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!("AST chunking requires the 'ast' feature to be enabled");
    }

    pub fn chunk_file(
        &mut self,
        _file_path: &std::path::Path,
        _content: &str,
    ) -> anyhow::Result<Vec<AstChunk>> {
        anyhow::bail!("AST chunking requires the 'ast' feature to be enabled");
    }
}

#[cfg(not(feature = "ast"))]
#[derive(Debug, Clone)]
pub struct AstChunk {
    pub name: String,
    pub content: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub object_type: String,
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
}
