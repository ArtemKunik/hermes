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
    let impl_chunk = chunks.iter().find(|c| c.node_type == NodeType::Impl);
    assert!(impl_chunk.is_some());
    assert_eq!(impl_chunk.unwrap().name, "MyStruct");
}

#[test]
fn chunk_rust_trait() {
    let code = "pub trait Searchable {\n    fn search(&self) -> Vec<String>;\n}\n";
    let chunks = chunk_rust(code);
    assert!(!chunks.is_empty());
    let trait_chunk = chunks.iter().find(|c| c.node_type == NodeType::Trait);
    assert!(trait_chunk.is_some(), "expected a Trait chunk");
    assert_eq!(trait_chunk.unwrap().name, "Searchable");
}

#[test]
fn extract_impl_name_simple() {
    assert_eq!(
        extract_impl_name("impl MyStruct {"),
        Some("MyStruct".to_string())
    );
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
    let summary = build_summary(
        "a_very_long_function_name_that_exceeds_eighty_characters_limit_for_sure",
        &NodeType::Function,
        long_line,
    );
    assert_eq!(
        summary,
        "function: a_very_long_function_name_that_exceeds_eighty_characters_limit_for_sure"
    );
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
