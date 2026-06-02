# Spec: Multi-language AST

## Language Extractor Trait

```rust
pub trait LanguageExtractor: Send + Sync {
    fn language(&self) -> &str;
    fn extensions(&self) -> &[&str];
    fn extract_symbols(&self, content: &[u8], file_path: &str) -> Vec<SymbolChunk>;
    fn extract_xrefs(&self, content: &[u8], file_path: &str) -> Vec<Xref>;
}
```

## TypeScript Extractor

### Symbols (tree-sitter node kinds)

| Node Kind | Mapped Kind | Exported Check |
|-----------|-------------|----------------|
| `function_declaration` | function | Check for `export` parent |
| `class_declaration` | class | Check for `export` parent |
| `interface_declaration` | interface | Check for `export` parent |
| `type_alias_declaration` | type | Check for `export` parent |
| `enum_declaration` | enum | Check for `export` parent |
| `lexical_declaration` (const) | const | Check for `export` keyword |
| `method_definition` | method | Always (within class) |

### Xrefs

| Pattern | XrefKind |
|---------|----------|
| `import { X } from '...'` | Imports (each named import) |
| `import X from '...'` | Imports (default import) |
| `call_expression` with `identifier` callee | Calls |
| `call_expression` with `member_expression` callee | Calls (last property) |

## Python Extractor

### Symbols (tree-sitter node kinds)

| Node Kind | Mapped Kind | Exported Check |
|-----------|-------------|----------------|
| `function_definition` | function | Not prefixed with `_` |
| `class_definition` | class | Not prefixed with `_` |
| `decorated_definition` | (inner kind) | Delegate to inner |

### Xrefs

| Pattern | XrefKind |
|---------|----------|
| `import_statement` | Imports (each dotted name's last segment) |
| `import_from_statement` | Imports (each name after `import`) |
| `call` with `identifier` or `attribute` | Calls |

## Parser Initialization

```rust
fn parser_for_extension(ext: &str) -> Option<(Parser, Box<dyn LanguageExtractor>)> {
    match ext {
        "rs" => Some((rust_parser(), Box::new(RustExtractor))),
        "ts" | "js" => Some((ts_parser(), Box::new(TypeScriptExtractor))),
        "tsx" | "jsx" => Some((tsx_parser(), Box::new(TypeScriptExtractor))),
        "py" => Some((py_parser(), Box::new(PythonExtractor))),
        _ => None,
    }
}
```

## Non-Goals

- Go, Java, Kotlin grammars (defer to future track)
- CSS/SCSS dependency analysis
- Framework-specific patterns (React hooks, Django models, etc.)
