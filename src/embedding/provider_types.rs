use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct EmbeddingRequest {
    pub(crate) model: String,
    pub(crate) content: EmbeddingContent,
}

#[derive(Debug, Serialize)]
pub(crate) struct EmbeddingContent {
    pub(crate) parts: Vec<EmbeddingPart>,
}

#[derive(Debug, Serialize)]
pub(crate) struct EmbeddingPart {
    pub(crate) text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingResponse {
    pub(crate) embedding: EmbeddingValues,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingValues {
    pub(crate) values: Vec<f32>,
}

#[derive(Serialize)]
pub(crate) struct GatewayRequest<'a> {
    pub(crate) model: &'a str,
    pub(crate) input: &'a str,
    pub(crate) caller_id: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct GatewayResponse {
    pub(crate) data: Vec<GatewayEmbedding>,
}

#[derive(Deserialize)]
pub(crate) struct GatewayEmbedding {
    pub(crate) embedding: Vec<f32>,
}

#[derive(Serialize)]
pub(crate) struct OaRequest<'a> {
    pub(crate) model: &'a str,
    pub(crate) input: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct OaResponse {
    pub(crate) data: Vec<EmbeddingResponse>,
}

#[derive(Serialize)]
pub(crate) struct LmStudioRequest<'a> {
    pub(crate) model: &'a str,
    pub(crate) input: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct LmStudioResponse {
    pub(crate) data: Vec<LmStudioEmbedding>,
}

#[derive(Deserialize)]
pub(crate) struct LmStudioEmbedding {
    pub(crate) embedding: Vec<f32>,
}
