use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalFact {
    pub id: String,
    pub project_id: String,
    pub node_id: Option<String>,
    pub fact_type: FactType,
    pub content: String,
    pub topic: Option<String>,
    pub tags: Vec<String>,
    pub confidence: Option<f64>,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub superseded_by: Option<String>,
    pub source_reference: Option<String>,
    pub provenance: Option<String>,
    pub repo_id: Option<String>,
    pub agent_id: Option<String>,
    pub stale: bool,
    pub delegated: bool,
}

pub struct AddFactInput<'a> {
    pub node_id: Option<&'a str>,
    pub fact_type: FactType,
    pub content: &'a str,
    pub topic: Option<&'a str>,
    pub tags: Vec<String>,
    pub confidence: Option<f64>,
    pub ttl: Option<&'a str>,
    pub source_reference: Option<&'a str>,
    pub provenance: Option<&'a str>,
    pub repo_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FactType {
    Architecture,
    ApiContract,
    Decision,
    ErrorPattern,
    Constraint,
    Learning,
    Assumption,
    Observation,
    Dependency,
}

impl FactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::ApiContract => "api_contract",
            Self::Decision => "decision",
            Self::ErrorPattern => "error_pattern",
            Self::Constraint => "constraint",
            Self::Learning => "learning",
            Self::Assumption => "assumption",
            Self::Observation => "observation",
            Self::Dependency => "dependency",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "architecture" => Self::Architecture,
            "api_contract" => Self::ApiContract,
            "decision" => Self::Decision,
            "error_pattern" => Self::ErrorPattern,
            "constraint" => Self::Constraint,
            "learning" => Self::Learning,
            "assumption" => Self::Assumption,
            "observation" => Self::Observation,
            "dependency" => Self::Dependency,
            _ => Self::Decision,
        }
    }
}

#[derive(Default)]
pub struct FactFilter {
    pub fact_type: Option<FactType>,
    pub topic: Option<String>,
    pub tags: Option<Vec<String>>,
    pub repo_id: Option<String>,
    pub node_id: Option<String>,
    pub limit: Option<u32>,
    pub include_expired: bool,
}
