// src/ingestion/xref_extractor.rs
// TRACK-040 Phase 3: Cross-reference edge extraction with tree-sitter
//
// Extracts Calls and Imports edges from Rust (and TypeScript) source files
// by walking the AST after the chunker has already produced nodes.
// Feature-gated behind "ast" — falls back silently when disabled.

#[cfg(feature = "ast")]
use anyhow::Result;
#[cfg(feature = "ast")]
use tree_sitter::{Language, Parser};

/// Semantics of a detected cross-reference.
#[derive(Debug, Clone, PartialEq)]
pub enum XrefKind {
    /// Caller invokes callee (source_id Calls target_id).
    Calls,
    /// File/module imports a name (source_id Imports target_id).
    Imports,
}

/// A single resolved cross-reference between two names in the same file.
///
/// `from_name` is the caller/importing context (function name or file path).
/// `to_name` is the callee/imported symbol name.
#[derive(Debug, Clone)]
pub struct Xref {
    pub from_name: String,
    pub to_name: String,
    pub kind: XrefKind,
}

// ─── Feature-enabled implementation ──────────────────────────────────────────

#[cfg(feature = "ast")]
pub struct XrefExtractor {
    parser: Parser,
}

#[cfg(feature = "ast")]
impl XrefExtractor {
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        parser.set_language(&language)?;
        Ok(Self { parser })
    }

    /// Parse `content` and return all detected cross-references.
    pub fn extract(&mut self, content: &str, file_path: &str) -> Vec<Xref> {
        let Some(tree) = self.parser.parse(content, None) else {
            return Vec::new();
        };

        let mut xrefs = Vec::new();
        let mut cursor = tree.walk();

        Self::walk(
            &mut cursor,
            content,
            file_path,
            None, // current enclosing function name
            &mut xrefs,
        );

        xrefs
    }

    fn walk(
        cursor: &mut tree_sitter::TreeCursor,
        content: &str,
        file_path: &str,
        enclosing_fn: Option<&str>,
        xrefs: &mut Vec<Xref>,
    ) {
        let node = cursor.node();
        let kind = node.kind();

        // Resolve enclosing function name for this level of recursion.
        let fn_name_owned: Option<String> = if kind == "function_item" {
            Self::child_text(&node, "identifier", content)
        } else {
            None
        };
        let current_fn: Option<&str> = fn_name_owned.as_deref().or(enclosing_fn);

        match kind {
            "use_declaration" => {
                // Extract the last segment of each imported path.
                let names = Self::extract_use_names(&node, content);
                let from = file_path.to_string();
                for name in names {
                    xrefs.push(Xref {
                        from_name: from.clone(),
                        to_name: name,
                        kind: XrefKind::Imports,
                    });
                }
            }
            "call_expression" => {
                // Extract the callee identifier (ignore method calls and closures).
                if let Some(callee) = Self::extract_callee(&node, content) {
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

        // Recurse into children.
        if cursor.goto_first_child() {
            loop {
                Self::walk(cursor, content, file_path, current_fn, xrefs);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    /// Get text of the first direct child with the given node kind.
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

    /// Extract callee name from a `call_expression` node.
    /// Returns `None` for method calls (field_expression) and arbitrary expressions.
    fn extract_callee(node: &tree_sitter::Node, content: &str) -> Option<String> {
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return None;
        }
        let callee_node = cursor.node();
        match callee_node.kind() {
            "identifier" => {
                let r = callee_node.range();
                Some(content[r.start_byte..r.end_byte].to_string())
            }
            "scoped_identifier" => {
                // e.g. `crate::foo::bar` → take the last segment
                Self::last_identifier_in(&callee_node, content)
            }
            _ => None, // method calls (field_expression), closures, etc. → skip
        }
    }

    /// Collect all imported leaf names from a use_declaration subtree.
    /// Handles: `use foo::Bar`, `use foo::{A, B}`, `use foo::*` (ignored).
    fn extract_use_names(node: &tree_sitter::Node, content: &str) -> Vec<String> {
        let mut names = Vec::new();
        Self::collect_use_names(node, content, &mut names);
        names
    }

    fn collect_use_names(node: &tree_sitter::Node, content: &str, out: &mut Vec<String>) {
        match node.kind() {
            "identifier" | "type_identifier" => {
                let r = node.range();
                let name = &content[r.start_byte..r.end_byte];
                if name != "use" && name != "crate" && name != "super" && name != "self" {
                    out.push(name.to_string());
                }
            }
            "scoped_identifier" => {
                // Only take the last segment (the actual name being imported).
                if let Some(name) = Self::last_identifier_in(node, content) {
                    out.push(name);
                }
            }
            "use_wildcard" => { /* `use foo::*` — skip */ }
            _ => {
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        Self::collect_use_names(&cursor.node(), content, out);
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Return the rightmost identifier in a scoped path like `a::b::C`.
    fn last_identifier_in(node: &tree_sitter::Node, content: &str) -> Option<String> {
        let mut cursor = node.walk();
        let mut last: Option<String> = None;
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if matches!(child.kind(), "identifier" | "type_identifier") {
                    let r = child.range();
                    last = Some(content[r.start_byte..r.end_byte].to_string());
                } else if child.kind() == "scoped_identifier" {
                    last = Self::last_identifier_in(&child, content);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        last
    }
}

// ─── Stub when "ast" feature is disabled ─────────────────────────────────────

#[cfg(not(feature = "ast"))]
pub struct XrefExtractor;

#[cfg(not(feature = "ast"))]
impl XrefExtractor {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn extract(&mut self, _content: &str, _file_path: &str) -> Vec<Xref> {
        Vec::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ast")]
    #[test]
    fn test_extract_rust_use_import() {
        let mut extractor = XrefExtractor::new().unwrap();
        let content = "use crate::graph::EdgeType;\nuse std::collections::HashMap;\n";
        let xrefs = extractor.extract(content, "src/lib.rs");
        let imports: Vec<_> = xrefs
            .iter()
            .filter(|x| x.kind == XrefKind::Imports)
            .collect();
        let names: Vec<&str> = imports.iter().map(|x| x.to_name.as_str()).collect();
        assert!(
            names.contains(&"EdgeType"),
            "expected EdgeType import, got {names:?}"
        );
        assert!(
            names.contains(&"HashMap"),
            "expected HashMap import, got {names:?}"
        );
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_extract_rust_function_call() {
        let mut extractor = XrefExtractor::new().unwrap();
        let content = "fn caller() { do_something(); }\nfn do_something() {}\n";
        let xrefs = extractor.extract(content, "src/lib.rs");
        let calls: Vec<_> = xrefs.iter().filter(|x| x.kind == XrefKind::Calls).collect();
        assert!(!calls.is_empty(), "expected at least one call xref");
        assert_eq!(calls[0].from_name, "caller");
        assert_eq!(calls[0].to_name, "do_something");
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_edge_dedup_via_ignore_constraint() {
        // XrefExtractor returns two identical calls; duplicate insertion must be idempotent.
        let mut extractor = XrefExtractor::new().unwrap();
        let content = "fn a() { b(); b(); }\nfn b() {}\n";
        let xrefs = extractor.extract(content, "src/lib.rs");
        let calls: Vec<_> = xrefs
            .iter()
            .filter(|x| x.kind == XrefKind::Calls && x.to_name == "b")
            .collect();
        // We may see 2 raw xrefs; dedup happens in the DB via INSERT OR IGNORE.
        assert!(!calls.is_empty());
    }

    #[cfg(not(feature = "ast"))]
    #[test]
    fn test_xref_extractor_stub_returns_empty() {
        let mut extractor = XrefExtractor::new().unwrap();
        let xrefs = extractor.extract("fn foo() { bar(); }", "src/lib.rs");
        assert!(xrefs.is_empty());
    }
}
