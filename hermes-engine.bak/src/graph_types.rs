// ChartApp/hermes-engine/src/graph_types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub node_type: NodeType,
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub summary: Option<String>,
    pub content_hash: Option<String>,
    /// Token count for the node's stored content (set during ingestion).
    pub content_tokens: Option<u64>,
    /// TRACK-040 Phase 2: AST object type (function, struct, etc.)
    pub object_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    File,
    Module,
    Function,
    Struct,
    Impl,
    Trait,
    Enum,
    Interface,
    Concept,
    Document,
    Config,
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Impl => "impl",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Concept => "concept",
            Self::Document => "document",
            Self::Config => "config",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "file" => Self::File,
            "module" => Self::Module,
            "function" => Self::Function,
            "struct" => Self::Struct,
            "impl" => Self::Impl,
            "trait" => Self::Trait,
            "enum" => Self::Enum,
            "interface" => Self::Interface,
            "concept" => Self::Concept,
            "document" => Self::Document,
            "config" => Self::Config,
            _ => Self::Concept,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub project_id: String,
    pub source_id: String,
    pub target_id: String,
    pub edge_type: EdgeType,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EdgeType {
    Calls,
    Imports,
    Implements,
    DependsOn,
    Contains,
    Documents,
    Defines,
    Uses,
    /// TRACK-045: test file/function → implementation node (test coverage mapping)
    Tests,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Implements => "implements",
            Self::DependsOn => "depends_on",
            Self::Contains => "contains",
            Self::Documents => "documents",
            Self::Defines => "defines",
            Self::Uses => "uses",
            Self::Tests => "tests",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "calls" => Self::Calls,
            "imports" => Self::Imports,
            "implements" => Self::Implements,
            "depends_on" => Self::DependsOn,
            "contains" => Self::Contains,
            "documents" => Self::Documents,
            "defines" => Self::Defines,
            "uses" => Self::Uses,
            "tests" => Self::Tests,
            _ => Self::DependsOn,
        }
    }
}
