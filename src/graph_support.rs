// tools/hermes-engine/src/graph_support.rs
use crate::graph_types::Node;
use anyhow::Result;

pub fn f32_slice_to_blob(slice: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(slice.len() * 4);
    for &f in slice {
        bytes.extend(&f.to_le_bytes());
    }
    bytes
}

pub fn blob_to_f32_vector(blob: &[u8]) -> Result<Vec<f32>> {
    if blob.len() % 4 != 0 {
        anyhow::bail!("invalid embedding blob length");
    }
    let mut v = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks(4) {
        let arr = [chunk[0], chunk[1], chunk[2], chunk[3]];
        v.push(f32::from_le_bytes(arr));
    }
    Ok(v)
}

pub trait OptionalRow {
    fn optional(self) -> std::result::Result<Option<Node>, rusqlite::Error>;
}

impl OptionalRow for std::result::Result<Node, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<Node>, rusqlite::Error> {
        match self {
            Ok(node) => Ok(Some(node)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
