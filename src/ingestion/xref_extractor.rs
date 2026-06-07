use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum XrefKind {
    Calls,
    Imports,
}

#[derive(Debug, Clone)]
pub struct Xref {
    pub from_name: String,
    pub to_name: String,
    pub kind: XrefKind,
}

#[cfg(feature = "ast")]
pub struct XrefExtractor;

#[cfg(feature = "ast")]
impl XrefExtractor {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn extract(&mut self, content: &str, file_path: &str) -> Vec<Xref> {
        let path = Path::new(file_path);
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let extractor = match crate::ingestion::lang::get_extractor(ext) {
            Some(e) => e,
            None => return Vec::new(),
        };
        extractor.extract_xrefs(content, file_path)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ast")]
    #[test]
    fn test_rust_use_import() {
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
    fn test_rust_function_call() {
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
    fn test_ts_import() {
        let mut extractor = XrefExtractor::new().unwrap();
        let content = "import { verifyToken } from './auth';";
        let xrefs = extractor.extract(content, "test.ts");
        assert!(
            xrefs.iter().any(|x| x.to_name == "verifyToken"),
            "expected import xref"
        );
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_py_import() {
        let mut extractor = XrefExtractor::new().unwrap();
        let content = "from mylib import helper\n";
        let xrefs = extractor.extract(content, "test.py");
        assert!(
            xrefs.iter().any(|x| x.to_name == "helper"),
            "expected import xref"
        );
    }

    #[cfg(feature = "ast")]
    #[test]
    fn test_unsupported_lang_returns_empty() {
        let mut extractor = XrefExtractor::new().unwrap();
        let xrefs = extractor.extract("fn foo() { bar(); }", "test.go");
        assert!(xrefs.is_empty());
    }

    #[cfg(not(feature = "ast"))]
    #[test]
    fn test_stub_returns_empty() {
        let mut extractor = XrefExtractor::new().unwrap();
        let xrefs = extractor.extract("fn foo() { bar(); }", "src/lib.rs");
        assert!(xrefs.is_empty());
    }
}
