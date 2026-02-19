use crate::graph::NodeType;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub name: String,
    pub node_type: NodeType,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub summary: String,
}

pub fn chunk_file(path: &Path, content: &str) -> Vec<Chunk> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "rs" => chunk_rust(content),
        "md" => chunk_markdown(content),
        "tsx" | "ts" | "jsx" | "js" => chunk_typescript(content),
        _ => chunk_whole_file(path, content),
    }
}

fn chunk_rust(content: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if let Some(chunk) = try_parse_rust_item(line, &lines, i) {
            chunks.push(chunk);
        }
        i += 1;
    }

    chunks
}

fn try_parse_rust_item(line: &str, lines: &[&str], start: usize) -> Option<Chunk> {
    let (name, node_type) = if line.starts_with("pub fn ")
        || line.starts_with("fn ")
        || line.starts_with("pub async fn ")
        || line.starts_with("async fn ")
    {
        (extract_fn_name(line)?, NodeType::Function)
    } else if line.starts_with("pub struct ") || line.starts_with("struct ") {
        (extract_after_keyword(line, "struct")?, NodeType::Struct)
    } else if line.starts_with("pub enum ") || line.starts_with("enum ") {
        (extract_after_keyword(line, "enum")?, NodeType::Enum)
    } else if line.starts_with("impl ") {
        (extract_impl_name(line)?, NodeType::Impl)
    } else if line.starts_with("pub trait ") || line.starts_with("trait ") {
        (extract_after_keyword(line, "trait")?, NodeType::Trait)
    } else {
        return None;
    };

    let end = find_block_end(lines, start);
    let block_content: String = lines[start..=end].join("\n");
    let summary = build_summary(&name, &node_type, lines[start]);

    Some(Chunk {
        name,
        node_type,
        content: block_content,
        start_line: start + 1,
        end_line: end + 1,
        summary,
    })
}

fn chunk_markdown(content: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut section_start: Option<(usize, String)> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("## ") || line.starts_with("# ") {
            if let Some((start, heading)) = section_start.take() {
                let section_content = lines[start..i].join("\n");
                chunks.push(Chunk {
                    name: heading.clone(),
                    node_type: NodeType::Document,
                    content: section_content,
                    start_line: start + 1,
                    end_line: i,
                    summary: heading,
                });
            }
            section_start = Some((i, line.trim_start_matches('#').trim().to_string()));
        }
    }

    if let Some((start, heading)) = section_start {
        let section_content = lines[start..].join("\n");
        chunks.push(Chunk {
            name: heading.clone(),
            node_type: NodeType::Document,
            content: section_content,
            start_line: start + 1,
            end_line: lines.len(),
            summary: heading,
        });
    }

    chunks
}

fn chunk_typescript(content: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if is_ts_function_start(trimmed) || is_ts_component_start(trimmed) {
            let name = extract_ts_name(trimmed).unwrap_or_else(|| format!("anonymous_{i}"));
            let end = find_block_end(&lines, i);
            let block_content = lines[i..=end].join("\n");
            chunks.push(Chunk {
                name: name.clone(),
                node_type: NodeType::Function,
                content: block_content,
                start_line: i + 1,
                end_line: end + 1,
                summary: format!("TypeScript function: {name}"),
            });
        }
    }

    chunks
}

fn chunk_whole_file(path: &Path, content: &str) -> Vec<Chunk> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    vec![Chunk {
        name: name.clone(),
        node_type: NodeType::File,
        content: content.to_string(),
        start_line: 1,
        end_line: content.lines().count(),
        summary: format!("File: {name}"),
    }]
}

fn extract_fn_name(line: &str) -> Option<String> {
    let after_fn = line.split("fn ").nth(1)?;
    let name = after_fn.split('(').next()?.split('<').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_after_keyword(line: &str, keyword: &str) -> Option<String> {
    let kw_with_space = format!("{keyword} ");
    let after = line.split(&kw_with_space).nth(1)?;
    let name = after
        .split('{')
        .next()?
        .split('<')
        .next()?
        .split('(')
        .next()?
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_impl_name(line: &str) -> Option<String> {
    let after_impl = line.strip_prefix("impl ")?;
    let name = after_impl
        .split('{')
        .next()?
        .split("for ")
        .last()?
        .split('<')
        .next()?
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn find_block_end(lines: &[&str], start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut found_open = false;

    for (i, line) in lines.iter().enumerate().skip(start) {
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
                found_open = true;
            } else if ch == '}' {
                depth -= 1;
            }
        }
        if found_open && depth <= 0 {
            return i;
        }
    }

    (start + 1).min(lines.len() - 1)
}

fn build_summary(name: &str, node_type: &NodeType, first_line: &str) -> String {
    let type_str = node_type.as_str();
    let clean_line = first_line.trim();
    if clean_line.len() > 80 {
        format!("{type_str}: {name}")
    } else {
        format!("{type_str}: {clean_line}")
    }
}

fn is_ts_function_start(line: &str) -> bool {
    (line.starts_with("export function ")
        || line.starts_with("function ")
        || line.starts_with("export const ")
        || line.starts_with("const "))
        && (line.contains("=>") || line.contains("("))
}

fn is_ts_component_start(line: &str) -> bool {
    line.starts_with("export default function ") || line.starts_with("export default class ")
}

fn extract_ts_name(line: &str) -> Option<String> {
    for keyword in &["function ", "const ", "class "] {
        if let Some(after) = line.split(keyword).nth(1) {
            let name = after
                .split('(')
                .next()?
                .split('=')
                .next()?
                .split(':')
                .next()?
                .split('<')
                .next()?
                .trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_rust_function() {
        let code = "pub fn hello(name: &str) -> String {\n    format!(\"Hello {name}\")\n}\n";
        let chunks = chunk_rust(code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "hello");
        assert_eq!(chunks[0].node_type, NodeType::Function);
    }

    #[test]
    fn chunk_rust_struct() {
        let code = "pub struct Config {\n    pub port: u16,\n}\n";
        let chunks = chunk_rust(code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "Config");
        assert_eq!(chunks[0].node_type, NodeType::Struct);
    }

    #[test]
    fn chunk_markdown_sections() {
        let md = "# Title\nIntro\n## Section A\nContent A\n## Section B\nContent B\n";
        let chunks = chunk_markdown(md);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].name, "Title");
    }

    #[test]
    fn extract_fn_name_variants() {
        assert_eq!(extract_fn_name("pub fn hello()"), Some("hello".to_string()));
        assert_eq!(extract_fn_name("fn main()"), Some("main".to_string()));
        assert_eq!(
            extract_fn_name("pub async fn fetch_data()"),
            Some("fetch_data".to_string())
        );
    }

    #[test]
    fn chunk_rust_enum() {
        let code = "pub enum Status {\n    Active,\n    Inactive,\n}\n";
        let chunks = chunk_rust(code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "Status");
        assert_eq!(chunks[0].node_type, NodeType::Enum);
    }

    #[test]
    fn chunk_rust_impl_block() {
        let code = "impl MyStruct {\n    pub fn method(&self) {}\n}\n";
        let chunks = chunk_rust(code);
        // impl block itself is a chunk
        let impl_chunk = chunks.iter().find(|c| c.node_type == NodeType::Impl);
        assert!(impl_chunk.is_some());
        assert_eq!(impl_chunk.unwrap().name, "MyStruct");
    }

    #[test]
    fn chunk_rust_trait() {
        let code = "pub trait Searchable {\n    fn search(&self) -> Vec<String>;\n}\n";
        let chunks = chunk_rust(code);
        // The chunker picks up both the trait block and the fn declaration inside it
        assert!(!chunks.is_empty());
        let trait_chunk = chunks.iter().find(|c| c.node_type == NodeType::Trait);
        assert!(trait_chunk.is_some(), "expected a Trait chunk");
        assert_eq!(trait_chunk.unwrap().name, "Searchable");
    }

    #[test]
    fn extract_impl_name_simple() {
        assert_eq!(extract_impl_name("impl MyStruct {"), Some("MyStruct".to_string()));
    }

    #[test]
    fn extract_impl_name_for_trait() {
        assert_eq!(
            extract_impl_name("impl Display for MyStruct {"),
            Some("MyStruct".to_string())
        );
    }

    #[test]
    fn chunk_file_dispatch_rust() {
        use std::path::PathBuf;
        let path = PathBuf::from("src/lib.rs");
        let code = "pub fn run() {\n    println!(\"hi\");\n}\n";
        let chunks = chunk_file(&path, code);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].node_type, NodeType::Function);
    }

    #[test]
    fn chunk_file_dispatch_markdown() {
        use std::path::PathBuf;
        let path = PathBuf::from("README.md");
        let md = "# Overview\nIntro\n## Usage\nDetails\n";
        let chunks = chunk_file(&path, md);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].node_type, NodeType::Document);
    }

    #[test]
    fn chunk_file_dispatch_unknown_extension() {
        use std::path::PathBuf;
        let path = PathBuf::from("config.toml");
        let content = "[package]\nname = \"hermes\"\n";
        let chunks = chunk_file(&path, content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].node_type, NodeType::File);
        assert_eq!(chunks[0].name, "config.toml");
    }

    #[test]
    fn chunk_typescript_function() {
        let code = "export function handleRequest(req: Request) {\n    return req;\n}\n";
        let chunks = chunk_typescript(code);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].name, "handleRequest");
        assert_eq!(chunks[0].node_type, NodeType::Function);
    }

    #[test]
    fn chunk_typescript_arrow_const() {
        let code = "const fetchData = async (url: string) => {\n    return fetch(url);\n};\n";
        let chunks = chunk_typescript(code);
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].name, "fetchData");
    }

    #[test]
    fn build_summary_short_line() {
        let summary = build_summary("my_fn", &NodeType::Function, "pub fn my_fn() {");
        assert_eq!(summary, "function: pub fn my_fn() {");
    }

    #[test]
    fn build_summary_long_line() {
        let long_line = "pub fn a_very_long_function_name_that_exceeds_eighty_characters_limit_for_sure(x: u32) {";
        let summary = build_summary("a_very_long_function_name_that_exceeds_eighty_characters_limit_for_sure", &NodeType::Function, long_line);
        assert_eq!(summary, "function: a_very_long_function_name_that_exceeds_eighty_characters_limit_for_sure");
    }

    #[test]
    fn chunk_whole_file_produces_single_chunk() {
        use std::path::PathBuf;
        let path = PathBuf::from("data.json");
        let content = "{\"key\": \"value\"}";
        let chunks = chunk_whole_file(&path, content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "data.json");
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn markdown_single_section() {
        let md = "# Only One\nSome content here\n";
        let chunks = chunk_markdown(md);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].name, "Only One");
    }

    #[test]
    fn markdown_empty_returns_empty() {
        let chunks = chunk_markdown("");
        assert!(chunks.is_empty());
    }
}
