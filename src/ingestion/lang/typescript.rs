use crate::ingestion::ast_chunker::AstChunk;
use crate::ingestion::lang::LanguageExtractor;
use crate::ingestion::xref_extractor::{Xref, XrefKind};

pub struct TypeScriptExtractor;

impl LanguageExtractor for TypeScriptExtractor {
    fn language(&self) -> &'static str {
        "typescript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "js", "jsx"]
    }

    fn extract_symbols(&self, content: &str, file_path: &str) -> Vec<AstChunk> {
        let ext = file_path.rsplit('.').next().unwrap_or("ts");
        let lang: tree_sitter::Language = match ext {
            "tsx" | "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut chunks = Vec::new();
        let mut cursor = tree.walk();
        ts_traverse_symbols(&mut cursor, content, file_path, &mut chunks);
        chunks
    }

    fn extract_xrefs(&self, content: &str, file_path: &str) -> Vec<Xref> {
        let ext = file_path.rsplit('.').next().unwrap_or("ts");
        let lang: tree_sitter::Language = match ext {
            "tsx" | "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(content, None) else {
            return Vec::new();
        };
        let mut xrefs = Vec::new();
        let mut cursor = tree.walk();
        ts_traverse_xrefs(&mut cursor, content, file_path, &mut xrefs);
        xrefs
    }
}

fn ts_traverse_symbols(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    chunks: &mut Vec<AstChunk>,
) {
    let node = cursor.node();
    if let Some(chunk) = ts_create_chunk(&node, content, file_path) {
        chunks.push(chunk);
    }
    if cursor.goto_first_child() {
        loop {
            ts_traverse_symbols(cursor, content, file_path, chunks);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn ts_create_chunk(node: &tree_sitter::Node, content: &str, file_path: &str) -> Option<AstChunk> {
    let (object_type, name_field) = match node.kind() {
        "function_declaration" => ("function", Some("name")),
        "class_declaration" => ("class", Some("name")),
        "interface_declaration" => ("interface", Some("name")),
        "type_alias_declaration" => ("type", Some("name")),
        "enum_declaration" => ("enum", Some("name")),
        "method_definition" => ("method", Some("name")),
        "lexical_declaration" => ("const", None),
        _ => return None,
    };

    let range = node.range();
    let name = if let Some(field) = name_field {
        node.child_by_field_name(field).and_then(|n| {
            let r = n.range();
            Some(content[r.start_byte..r.end_byte].to_string())
        })?
    } else {
        let mut c = node.walk();
        let mut found = None;
        if c.goto_first_child() {
            loop {
                let child = c.node();
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let r = name_node.range();
                        found = Some(content[r.start_byte..r.end_byte].to_string());
                        break;
                    }
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        found?
    };

    Some(AstChunk {
        name,
        content: content[range.start_byte..range.end_byte].to_string(),
        file_path: file_path.to_string(),
        start_line: range.start_point.row + 1,
        end_line: range.end_point.row + 1,
        object_type: object_type.to_string(),
    })
}

fn ts_traverse_xrefs(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    file_path: &str,
    xrefs: &mut Vec<Xref>,
) {
    let node = cursor.node();
    match node.kind() {
        "import_statement" => {
            for name in ts_extract_import_names(&node, content) {
                xrefs.push(Xref {
                    from_name: file_path.to_string(),
                    to_name: name,
                    kind: XrefKind::Imports,
                });
            }
        }
        "call_expression" => {
            if let Some(callee) = ts_extract_callee(&node, content) {
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
            ts_traverse_xrefs(cursor, content, file_path, xrefs);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn ts_extract_import_names(node: &tree_sitter::Node, content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                "import_clause" => {
                    ts_collect_import_clause(&child, content, &mut names);
                }
                "namespace_import" => {
                    if let Some(alias) = child.child_by_field_name("alias") {
                        let r = alias.range();
                        names.push(content[r.start_byte..r.end_byte].to_string());
                    }
                }
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    names
}

fn ts_collect_import_clause(node: &tree_sitter::Node, content: &str, names: &mut Vec<String>) {
    // Default import: `import Foo from ...` → identifier child
    if let Some(id) = node.child_by_field_name("name") {
        let r = id.range();
        names.push(content[r.start_byte..r.end_byte].to_string());
    }
    // Named imports: `import { Foo, Bar }` → named_imports child
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "named_imports" {
                let mut nc = child.walk();
                if nc.goto_first_child() {
                    loop {
                        let nchild = nc.node();
                        if nchild.kind() == "import_specifier" {
                            if let Some(name) = nchild.child_by_field_name("name") {
                                let r = name.range();
                                names.push(content[r.start_byte..r.end_byte].to_string());
                            }
                        }
                        if !nc.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn ts_extract_callee(node: &tree_sitter::Node, content: &str) -> Option<String> {
    if let Some(func) = node.child_by_field_name("function") {
        match func.kind() {
            "identifier" => {
                let r = func.range();
                return Some(content[r.start_byte..r.end_byte].to_string());
            }
            "member_expression" => {
                if let Some(prop) = func.child_by_field_name("property") {
                    let r = prop.range();
                    return Some(content[r.start_byte..r.end_byte].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
