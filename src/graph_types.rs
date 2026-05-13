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
    Concept,
    Document,
    // hermes-mind personal knowledge types
    Message,
    Email,
    Contact,
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
            Self::Concept => "concept",
            Self::Document => "document",
            Self::Message => "message",
            Self::Email => "email",
            Self::Contact => "contact",
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
            "concept" => Self::Concept,
            "document" => Self::Document,
            "message" => Self::Message,
            "email" => Self::Email,
            "contact" => Self::Contact,
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
            _ => Self::DependsOn,
        }
    }
}

/// Data for one changed chunk, passed to [`KnowledgeGraph::ingest_file_batch`].
pub struct ChunkWriteRecord {
    pub node: Node,
    pub content: String,
    pub edge: Edge,
    pub hash_key: String,
    pub hash_value: String,
}
