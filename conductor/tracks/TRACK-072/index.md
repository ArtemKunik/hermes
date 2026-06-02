# TRACK-072: Multi-language AST (Rust + TypeScript + Python)

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Ingestion
**Depends on**: TRACK-069 (symbol_index table to populate for new languages)
**Branch**: `feat/multi-lang-ast`

## Goal

Wire up the already-declared `tree-sitter-typescript` and `tree-sitter-python` dependencies to extract symbols and cross-references from TypeScript/JavaScript and Python files, matching the existing Rust extraction quality.

## Problem

Hermes currently extracts symbols and dependencies only from Rust files (via tree-sitter). TypeScript/JavaScript and Python files get regex-based chunking which misses imports, cross-references, and fine-grained symbol metadata. The tree-sitter grammars are already declared in `Cargo.toml` but never instantiated.

## What to Build

- `src/ingestion/lang/` module with `LanguageExtractor` trait
- Refactor existing Rust extraction into `lang/rust.rs`
- New `lang/typescript.rs`: function, class, interface, type, enum, const extraction + import xrefs
- New `lang/python.rs`: def, class, async def extraction + import xrefs
- Expand `ast_chunker.rs` and `xref_extractor.rs` to dispatch via language registry

## Acceptance Criteria

- [x] `LanguageExtractor` trait defined with `extract_symbols()` + `extract_xrefs()`
- [x] Rust extraction refactored into `lang/rust.rs` (no behavior change)
- [x] TypeScript extractor extracts: function, class, interface, type, enum, exported const
- [x] TypeScript extractor extracts xrefs: `import` statements, function calls
- [x] Python extractor extracts: def, class, async def
- [x] Python extractor extracts xrefs: `import`/`from...import`, function calls
- [x] `ast_chunker.rs` dispatches to language registry
- [x] `xref_extractor.rs` dispatches to language registry
- [x] `is_ast_supported_file()` includes .ts/.tsx/.js/.jsx/.py
- [x] Symbols populated in `symbol_index` for all three languages
- [x] All existing Rust tests still pass

## Links

- [Specification](./spec.md)
- [Implementation Plan](./plan.md)
