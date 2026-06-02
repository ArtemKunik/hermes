use crate::ingestion::ast_chunker::AstChunk;
use crate::ingestion::xref_extractor::Xref;

pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn extract_symbols(&self, content: &str, file_path: &str) -> Vec<AstChunk>;
    fn extract_xrefs(&self, content: &str, file_path: &str) -> Vec<Xref>;
}

pub mod rust;
pub mod typescript;
pub mod python;

pub fn get_extractor(ext: &str) -> Option<Box<dyn LanguageExtractor>> {
    match ext {
        "rs" => Some(Box::new(rust::RustExtractor)),
        "ts" | "tsx" | "js" | "jsx" => Some(Box::new(typescript::TypeScriptExtractor)),
        "py" => Some(Box::new(python::PythonExtractor)),
        _ => None,
    }
}

pub fn parser_for_extension(ext: &str) -> Option<(tree_sitter::Parser, Box<dyn LanguageExtractor>)> {
    let extractor = get_extractor(ext)?;
    let mut parser = tree_sitter::Parser::new();
    let language: tree_sitter::Language = match ext {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" | "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "js" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "py" => tree_sitter_python::LANGUAGE.into(),
        _ => return None,
    };
    if parser.set_language(&language).is_err() {
        return None;
    }
    Some((parser, extractor))
}
