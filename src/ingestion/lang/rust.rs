use crate::ingestion::ast_chunker::AstChunk;
use crate::ingestion::lang::LanguageExtractor;
use crate::ingestion::xref_extractor::{Xref, XrefKind};

pub struct RustExtractor;

impl LanguageExtractor for RustExtractor {
    fn language(&self) -> &'static str {
        "rust"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn extract_symbols(&self, content: &str, file_path: &str) -> Vec<AstChunk> {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut chunks = Vec::new();
        let mut cursor = tree.walk();
        traverse_symbols(&mut cursor, content, file_path, &mut chunks);
        chunks
    }

    fn extract_xrefs(&self, content: &str, file_path: &str) -> Vec<Xref> {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut xrefs = Vec::new();
        let mut cursor = tree.walk();
        traverse_xrefs(&mut cursor, content, file_path, None, &mut xrefs);
        xrefs
    }
}

fn traverse_symbols(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    chunks: &mut Vec<AstChunk>,
) {
    let node = cursor.node();
    if let Some(chunk) = create_rust_chunk(&node, content, file_path) {
        chunks.push(chunk);
    }
    if cursor.goto_first_child() {
        loop {
            traverse_symbols(cursor, content, file_path, chunks);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn create_rust_chunk(node: &tree_sitter::Node, content: &str, file_path: &str) -> Option<AstChunk> {
    let object_type = match node.kind() {
        "function_item" => "function",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "impl_item" => "impl",
        "trait_item" => "trait",
        "mod_item" => "module",
        _ => return None,
    };
    let range = node.range();
    let name = extract_rust_name(node, content)?;
    let chunk_content = &content[range.start_byte..range.end_byte];
    Some(AstChunk {
        name,
        content: chunk_content.to_string(),
        file_path: file_path.to_string(),
        start_line: range.start_point.row + 1,
        end_line: range.end_point.row + 1,
        object_type: object_type.to_string(),
    })
}

fn extract_rust_name(node: &tree_sitter::Node, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if matches!(child.kind(), "identifier" | "type_identifier") {
                let r = child.range();
                return Some(content[r.start_byte..r.end_byte].to_string());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn traverse_xrefs(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    enclosing_fn: Option<&str>,
    xrefs: &mut Vec<Xref>,
) {
    let node = cursor.node();
    let kind = node.kind();

    let fn_name = if kind == "function_item" {
        child_text(&node, "identifier", content)
    } else {
        None
    };
    let current_fn: Option<&str> = fn_name.as_deref().or(enclosing_fn);

    match kind {
        "use_declaration" => {
            let names = extract_use_names(&node, content);
            for name in names {
                xrefs.push(Xref {
                    from_name: file_path.to_string(),
                    to_name: name,
                    kind: XrefKind::Imports,
                });
            }
        }
        "call_expression" => {
            if let Some(callee) = extract_rust_callee(&node, content) {
                let from = current_fn.unwrap_or(file_path).to_string();
                xrefs.push(Xref {
                    from_name: from,
                    to_name: callee,
                    kind: XrefKind::Calls,
                });
            }
        }
        _ => {}
    }

    if cursor.goto_first_child() {
        loop {
            traverse_xrefs(cursor, content, file_path, current_fn, xrefs);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn child_text(node: &tree_sitter::Node, child_kind: &str, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == child_kind {
                let r = child.range();
                return Some(content[r.start_byte..r.end_byte].to_string());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

fn extract_rust_callee(node: &tree_sitter::Node, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    let callee = cursor.node();
    match callee.kind() {
        "identifier" => {
            let r = callee.range();
            Some(content[r.start_byte..r.end_byte].to_string())
        }
        "scoped_identifier" => last_identifier_in(&callee, content),
        _ => None,
    }
}

fn last_identifier_in(node: &tree_sitter::Node, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut last = None;
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if matches!(child.kind(), "identifier" | "type_identifier") {
                let r = child.range();
                last = Some(content[r.start_byte..r.end_byte].to_string());
            } else if child.kind() == "scoped_identifier" {
                last = last_identifier_in(&child, content);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    last
}

fn extract_use_names(node: &tree_sitter::Node, content: &str) -> Vec<String> {
    let mut names = Vec::new();
    collect_use_names(node, content, &mut names);
    names
}

fn collect_use_names(node: &tree_sitter::Node, content: &str, out: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "type_identifier" => {
            let r = node.range();
            let name = &content[r.start_byte..r.end_byte];
            if !matches!(name, "use" | "crate" | "super" | "self") {
                out.push(name.to_string());
            }
        }
        "scoped_identifier" => {
            if let Some(name) = last_identifier_in(node, content) {
                out.push(name);
            }
        }
        "use_wildcard" => {}
        _ => {
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    collect_use_names(&cursor.node(), content, out);
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
