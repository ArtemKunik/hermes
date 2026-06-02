# Plan: TRACK-072 — Multi-language AST

## Phase 1 — Language Registry

- [ ] Create `src/ingestion/lang/mod.rs` with `LanguageExtractor` trait
- [ ] Implement `get_extractor(extension: &str) -> Option<Box<dyn LanguageExtractor>>`
- [ ] Implement `parser_for_extension()` with tree-sitter parser initialization

## Phase 2 — Rust Refactor

- [ ] Create `src/ingestion/lang/rust.rs` — extract logic from `ast_chunker.rs`
- [ ] Move symbol extraction (function_item, struct_item, etc.) into `RustExtractor`
- [ ] Move xref extraction (use_declaration, call_expression) into `RustExtractor`
- [ ] Refactor `ast_chunker.rs` to dispatch via `lang::get_extractor()`
- [ ] Refactor `xref_extractor.rs` to dispatch via `lang::get_extractor()`
- [ ] Verify all existing Rust tests pass (no behavior change)

## Phase 3 — TypeScript Extractor

- [ ] Create `src/ingestion/lang/typescript.rs`
- [ ] Implement symbol extraction: function, class, interface, type, enum, const
- [ ] Implement exported detection (export keyword parent)
- [ ] Implement xref extraction: import statements, call expressions
- [ ] Handle both TS and TSX parser variants
- [ ] Add test fixtures: sample .ts and .tsx files

## Phase 4 — Python Extractor

- [ ] Create `src/ingestion/lang/python.rs`
- [ ] Implement symbol extraction: function_definition, class_definition
- [ ] Implement exported detection (no leading underscore)
- [ ] Implement xref extraction: import_statement, import_from_statement, call
- [ ] Add test fixtures: sample .py files

## Phase 5 — Integration

- [ ] Expand `is_ast_supported_file()` in `src/ingestion/mod.rs`
- [ ] Expand `file_ops.rs` to dispatch AST chunking for TS/Python files
- [ ] Verify symbol_index populated for TS/Python after ingestion
- [ ] Integration test: ingest mixed-language project, verify symbols + xrefs

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| tree-sitter-typescript API differences from rust | Medium | Abstract behind trait, handle per-language |
| TSX vs TS parser confusion | Low | Route by extension: .tsx/.jsx → TSX, .ts/.js → TS |
| Python decorator handling complex | Low | Start simple, handle decorated_definition as wrapper |

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/ingestion/lang/mod.rs` | **NEW** — trait + dispatcher | Low |
| `src/ingestion/lang/rust.rs` | **NEW** — extracted from existing | Medium |
| `src/ingestion/lang/typescript.rs` | **NEW** — TS/JS extractor | Medium |
| `src/ingestion/lang/python.rs` | **NEW** — Python extractor | Medium |
| `src/ingestion/ast_chunker.rs` | Refactor to dispatch | Medium |
| `src/ingestion/xref_extractor.rs` | Refactor to dispatch | Medium |
| `src/ingestion/mod.rs` | Expand language support check | Low |
| `src/ingestion/file_ops.rs` | Dispatch for TS/Python | Low |
