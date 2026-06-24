use crate::graph::{EdgeType, KnowledgeGraph, NodeType};
use crate::ingestion::env_scanner::{DiscoveredEnvVar, EnvScanner};
use crate::ingestion::hash_tracker;
use crate::ingestion::llm_enricher::LlmEnricher;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn ingest_file_inner(
    graph: &KnowledgeGraph,
    ht: &hash_tracker::HashTracker<'_>,
    enricher: Option<&Arc<LlmEnricher>>,
    env_scanner: &EnvScanner,
    file_path: &Path,
    env_vars_acc: &Arc<Mutex<Vec<DiscoveredEnvVar>>>,
) -> Result<usize> {
    // Read as raw bytes and decode lossily so non-UTF-8 files (Latin-1, etc.)
    // are ingested with replacement characters rather than silently skipped.
    let raw_bytes = std::fs::read(file_path)?;
    let content = String::from_utf8_lossy(&raw_bytes).into_owned();
    let path_str = hash_tracker::normalize_logical_path(&file_path.to_string_lossy());

    // TRACK-040 Phase 2: Use AST chunking if available, otherwise fall back to regex.
    #[cfg(feature = "ast")]
    let (ast_chunks, regex_chunks) =
        if file_path.extension().and_then(|s| s.to_str()) == Some("rs") {
            match crate::ingestion::ast_chunker::AstChunker::new()
                .and_then(|mut c| c.chunk_file(file_path, &content))
            {
                Ok(chunks) => (chunks, Vec::new()),
                Err(_) => (
                    Vec::new(),
                    crate::ingestion::chunker::chunk_file(file_path, &content),
                ),
            }
        } else {
            (
                Vec::new(),
                crate::ingestion::chunker::chunk_file(file_path, &content),
            )
        };

    #[cfg(not(feature = "ast"))]
    let (ast_chunks, regex_chunks) = (
        Vec::<crate::ingestion::ast_chunker::AstChunk>::new(),
        crate::ingestion::chunker::chunk_file(file_path, &content),
    );

    let file_hash = hash_tracker::compute_hash(&content);
    let file_node = graph
        .create_node_builder()
        .name(&path_str)
        .node_type(NodeType::File)
        .file_path(&path_str)
        .lines(1, content.lines().count() as i64)
        .content_hash(&file_hash)
        .content_tokens(crate::search::estimate_tokens(&content))
        .build();

    graph.add_node(&file_node)?;
    graph.index_fts(&file_node, &content)?;

    let mut created = 1;

    for ast_chunk in &ast_chunks {
        let chunk_key = format!("{}::{}", path_str, ast_chunk.name);
        let chunk_hash = hash_tracker::compute_hash(&ast_chunk.content);
        if ht.is_chunk_unchanged(&chunk_key, &chunk_hash)? {
            continue;
        }

        let summary = enricher
            .as_ref()
            .and_then(|e| e.enrich(&path_str, &ast_chunk.name, &ast_chunk.content))
            .unwrap_or_else(|| format!("{}: {}", ast_chunk.object_type, ast_chunk.name));

        let node_type = match ast_chunk.object_type.as_str() {
            "function" => NodeType::Function,
            "struct" => NodeType::Struct,
            "enum" => NodeType::Enum,
            "trait" => NodeType::Trait,
            "impl" => NodeType::Impl,
            "module" => NodeType::Module,
            _ => NodeType::Concept,
        };

        let chunk_node = graph
            .create_node_builder()
            .name(&ast_chunk.name)
            .node_type(node_type)
            .file_path(&path_str)
            .lines(ast_chunk.start_line as i64, ast_chunk.end_line as i64)
            .summary(&summary)
            .object_type(&ast_chunk.object_type)
            .content_tokens(crate::search::estimate_tokens(&ast_chunk.content))
            .build();

        graph.add_node(&chunk_node)?;
        graph.index_fts(&chunk_node, &ast_chunk.content)?;
        let _ = maybe_index_embedding_inner(graph, &path_str, &ast_chunk.name, &ast_chunk.content);

        let edge = graph
            .create_edge_builder()
            .source(&file_node.id)
            .target(&chunk_node.id)
            .edge_type(EdgeType::Contains)
            .build();
        graph.add_edge(&edge)?;
        ht.update_chunk_hash(&chunk_key, &chunk_hash)?;
        created += 1;
    }

    for chunk in &regex_chunks {
        let chunk_key = format!("{}::{}", path_str, chunk.name);
        let chunk_hash = hash_tracker::compute_hash(&chunk.content);
        if ht.is_chunk_unchanged(&chunk_key, &chunk_hash)? {
            continue;
        }

        let summary = enricher
            .as_ref()
            .and_then(|e| e.enrich(&path_str, &chunk.name, &chunk.content))
            .unwrap_or_else(|| chunk.summary.clone());

        let chunk_node = graph
            .create_node_builder()
            .name(&chunk.name)
            .node_type(chunk.node_type.clone())
            .file_path(&path_str)
            .lines(chunk.start_line as i64, chunk.end_line as i64)
            .summary(&summary)
            .content_tokens(crate::search::estimate_tokens(&chunk.content))
            .build();

        graph.add_node(&chunk_node)?;
        graph.index_fts(&chunk_node, &chunk.content)?;
        let _ = maybe_index_embedding_inner(graph, &path_str, &chunk.name, &chunk.content);

        let edge = graph
            .create_edge_builder()
            .source(&file_node.id)
            .target(&chunk_node.id)
            .edge_type(EdgeType::Contains)
            .build();
        graph.add_edge(&edge)?;
        ht.update_chunk_hash(&chunk_key, &chunk_hash)?;
        created += 1;
    }

    // Scan for environment variables and create Config nodes/edges.
    let cleaned_content = EnvScanner::strip_comments(&content);
    let discovered_vars = env_scanner.scan_file(file_path, &cleaned_content);
    for var in &discovered_vars {
        let config_node_id = format!("{}:config:{}", graph.project_id(), var.name);
        let config_node = graph
            .create_node_builder()
            .name(&var.name)
            .node_type(crate::graph::NodeType::Config)
            .summary(&format!("Environment variable: {}", var.name))
            .build();
        let mut config_node = config_node;
        config_node.id = config_node_id.clone();
        graph.add_node(&config_node)?;

        let edge_type = if var.is_definition {
            crate::graph::EdgeType::Defines
        } else {
            crate::graph::EdgeType::Uses
        };
        let edge = graph
            .create_edge_builder()
            .source(&file_node.id)
            .target(&config_node_id)
            .edge_type(edge_type)
            .build();
        graph.add_edge(&edge)?;
    }
    if !discovered_vars.is_empty() {
        if let Ok(mut acc) = env_vars_acc.lock() {
            acc.extend(discovered_vars);
        }
    }

    Ok(created)
}

/// Attempt to generate and store an embedding for a symbol chunk.
/// Best-effort: failures are ignored so indexing continues if the
/// embedding service is unavailable.
pub fn maybe_index_embedding_inner(
    graph: &KnowledgeGraph,
    file_path: &str,
    symbol_name: &str,
    content: &str,
) -> Result<()> {
    let mut lines = content.lines();
    let signature = lines.next().unwrap_or("").trim().to_string();
    let snippet: String = content.lines().take(3).collect::<Vec<_>>().join("\n");
    let preview = format!("{}\n{}", signature, snippet);

    let embedding = crate::embedding::embed_text(&preview);
    if embedding.is_empty() {
        return Ok(());
    }

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(file_path.as_bytes());
    hasher.update(b"::");
    hasher.update(symbol_name.as_bytes());
    let id = hex::encode(hasher.finalize());

    let language = match Path::new(file_path).extension().and_then(|s| s.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("py") => "python",
        _ => "unknown",
    };

    graph.upsert_symbol_embedding(
        &id,
        symbol_name,
        file_path,
        language,
        &signature,
        &snippet,
        &embedding,
    )?;
    Ok(())
}

/// Backfill stored `content_tokens` for nodes that are missing the value.
/// Returns the number of nodes updated.
pub fn backfill_content_tokens_inner(graph: &KnowledgeGraph) -> Result<usize> {
    let nodes = graph.get_all_nodes()?;
    let mut updated: usize = 0;

    for node in nodes {
        if node.content_tokens.unwrap_or(0) > 0 {
            continue;
        }
        let Some(ref path) = node.file_path else {
            continue;
        };
        let file_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let start = node.start_line.unwrap_or(1).max(1) as usize;
        let end = node.end_line.unwrap_or(0) as usize;
        let content = if end == 0 {
            file_content.clone()
        } else {
            let lines: Vec<&str> = file_content.lines().collect();
            let start_idx = (start - 1).min(lines.len());
            let end_idx = end.min(lines.len());
            lines[start_idx..end_idx].join("\n")
        };
        let tokens = crate::search::estimate_tokens(&content);
        graph.update_node_content_tokens(&node.id, tokens)?;
        updated += 1;
    }

    Ok(updated)
}
