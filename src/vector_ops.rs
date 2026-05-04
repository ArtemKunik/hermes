use crate::graph::Node;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const VECTOR_DIMENSION: usize = 256;

pub fn combined_node_text(node: &Node) -> String {
    let mut text = String::new();
    text.push_str(&node.name);
    if let Some(summary) = &node.summary {
        text.push(' ');
        text.push_str(summary);
    }
    if let Some(path) = &node.file_path {
        text.push(' ');
        text.push_str(path);
    }
    text
}

pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| part.len() > 1)
        .collect()
}

pub fn build_vector(tokens: &[String]) -> Vec<f32> {
    let mut vec = vec![0.0f32; VECTOR_DIMENSION];
    for token in tokens {
        let index = stable_hash(token) % VECTOR_DIMENSION;
        vec[index] += 1.0;
    }
    normalize(&mut vec);
    vec
}

pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn stable_hash(value: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish() as usize
}

fn normalize(vec: &mut [f32]) {
    let norm = vec
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    if norm < f64::EPSILON {
        return;
    }
    for value in vec {
        *value /= norm as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_ignores_short_tokens() {
        let tokens = tokenize("fn a fetch_exchange_rate");
        assert!(tokens.contains(&"fetch_exchange_rate".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn build_vector_has_correct_dimension() {
        let v = build_vector(&tokenize("hello world"));
        assert_eq!(v.len(), VECTOR_DIMENSION);
    }

    #[test]
    fn vec_to_blob_blob_to_vec_roundtrip() {
        let v: Vec<f32> = (0..VECTOR_DIMENSION).map(|i| i as f32 * 0.01).collect();
        let blob = vec_to_blob(&v);
        let recovered = blob_to_vec(&blob);
        assert_eq!(v.len(), recovered.len());
        for (a, b) in v.iter().zip(recovered.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }
}
