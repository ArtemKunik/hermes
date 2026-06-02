use crate::ingestion::ast_chunker::AstChunk;
use crate::ingestion::lang::LanguageExtractor;
use crate::ingestion::xref_extractor::{Xref, XrefKind};

pub struct PythonExtractor;

impl LanguageExtractor for PythonExtractor {
    fn language(&self) -> &'static str {
        "python"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["py"]
    }

    fn extract_symbols(&self, content: &str, file_path: &str) -> Vec<AstChunk> {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut chunks = Vec::new();
        let mut cursor = tree.walk();
        py_traverse_symbols(&mut cursor, content, file_path, &mut chunks);
        chunks
    }

    fn extract_xrefs(&self, content: &str, file_path: &str) -> Vec<Xref> {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .is_err()
        {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut xrefs = Vec::new();
        let mut cursor = tree.walk();
        py_traverse_xrefs(&mut cursor, content, file_path, &mut xrefs);
        xrefs
    }
}

fn py_traverse_symbols(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    chunks: &mut Vec<AstChunk>,
) {
    let node = cursor.node();
    match node.kind() {
        "function_definition" | "class_definition" => {
            if let Some(chunk) = py_create_chunk_def(&node, content, file_path) {
                chunks.push(chunk);
            }
        }
        "decorated_definition" => {
            // Find the inner function/class definition.
            let mut c = node.walk();
            if c.goto_first_child() {
                loop {
                    let child = c.node();
                    if matches!(child.kind(), "function_definition" | "class_definition") {
                        if let Some(chunk) = py_create_chunk_def(&child, content, file_path) {
                            chunks.push(chunk);
                        }
                        break;
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {}
    }
    if cursor.goto_first_child() {
        loop {
            py_traverse_symbols(cursor, content, file_path, chunks);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn py_create_chunk_def(
    node: &tree_sitter::Node,
    content: &str,
    file_path: &str,
) -> Option<AstChunk> {
    let object_type = match node.kind() {
        "function_definition" => "function",
        "class_definition" => "class",
        _ => return None,
    };
    let name = node.child_by_field_name("name").and_then(|n| {
        let r = n.range();
        Some(content[r.start_byte..r.end_byte].to_string())
    })?;
    let range = node.range();
    Some(AstChunk {
        name,
        content: content[range.start_byte..range.end_byte].to_string(),
        file_path: file_path.to_string(),
        start_line: range.start_point.row + 1,
        end_line: range.end_point.row + 1,
        object_type: object_type.to_string(),
    })
}

fn py_traverse_xrefs(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    xrefs: &mut Vec<Xref>,
) {
    let node = cursor.node();
    match node.kind() {
        "import_statement" => {
            for name in py_extract_import_names(&node, content) {
                xrefs.push(Xref {
                    from_name: file_path.to_string(),
                    to_name: name,
                    kind: XrefKind::Imports,
                });
            }
        }
        "import_from_statement" => {
            for name in py_extract_from_names(&node, content) {
                xrefs.push(Xref {
                    from_name: file_path.to_string(),
                    to_name: name,
                    kind: XrefKind::Imports,
                });
            }
        }
        "call" => {
            if let Some(callee) = py_extract_callee(&node, content) {
                xrefs.push(Xref {
                    from_name: file_path.to_string(),
                    to_name: callee,
                    kind: XrefKind::Calls,
                });
            }
        }
        _ => {}
    }
    if cursor.goto_first_child() {
        loop {
            py_traverse_xrefs(cursor, content, file_path, xrefs);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn py_extract_import_names(node: &tree_sitter::Node, content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "dotted_name" {
                let mut dc = child.walk();
                let mut last = None;
                if dc.goto_first_child() {
                    loop {
                        let dchild = dc.node();
                        if dchild.kind() == "identifier" {
                            let r = dchild.range();
                            last = Some(content[r.start_byte..r.end_byte].to_string());
                        }
                        if !dc.goto_next_sibling() {
                            break;
                        }
                    }
                }
                if let Some(name) = last {
                    names.push(name);
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    names
}

fn py_extract_from_names(node: &tree_sitter::Node, content: &str) -> Vec<String> {
    let mut names = Vec::new();
    // `from X import Y` — Y is the child with field "name"
    if let Some(named) = node.child_by_field_name("name") {
        match named.kind() {
            "dotted_name" => {
                let mut dc = named.walk();
                let mut last = None;
                if dc.goto_first_child() {
                    loop {
                        let dchild = dc.node();
                        if dchild.kind() == "identifier" {
                            let r = dchild.range();
                            last = Some(content[r.start_byte..r.end_byte].to_string());
                        }
                        if !dc.goto_next_sibling() {
                            break;
                        }
                    }
                }
                if let Some(n) = last {
                    names.push(n);
                }
            }
            "aliased_import" => {
                if let Some(n) = named.child_by_field_name("name") {
                    let r = n.range();
                    names.push(content[r.start_byte..r.end_byte].to_string());
                }
            }
            _ => {}
        }
    }
    names
}

fn py_extract_callee(node: &tree_sitter::Node, content: &str) -> Option<String> {
    if let Some(func) = node.child_by_field_name("function") {
        match func.kind() {
            "identifier" => {
                let r = func.range();
                return Some(content[r.start_byte..r.end_byte].to_string());
            }
            "attribute" => {
                if let Some(attr) = func.child_by_field_name("attribute") {
                    let r = attr.range();
                    return Some(content[r.start_byte..r.end_byte].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
