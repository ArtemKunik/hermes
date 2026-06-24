use std::env;

pub(crate) const DEFAULT_AOSTAR_LMSTUDIO_URL: &str = "http://aostar-maco:8001";
pub(crate) const DEFAULT_LEGION_LMSTUDIO_URL: &str = "http://127.0.0.1:1234";
pub(crate) const DEFAULT_LMSTUDIO_MODEL: &str = "qwen3-embedding";
const DEFAULT_LEGION_LMSTUDIO_MODEL: &str = "nomic-ai/text-embedding-nomic-embed-text-v1.5";

#[derive(Clone)]
pub(crate) struct LmStudioTarget {
    pub(crate) url: String,
    pub(crate) model: String,
}

pub(crate) fn resolve_lmstudio_targets() -> Vec<LmStudioTarget> {
    let configured_model = configured_lmstudio_model();
    if let Some(url) = http_env("LMSTUDIO_EMBED_URL").or_else(|| http_env("LMSTUDIO_URL")) {
        return vec![LmStudioTarget {
            url,
            model: configured_model.unwrap_or_else(|| DEFAULT_LMSTUDIO_MODEL.to_string()),
        }];
    }

    if is_legion_machine() {
        return vec![
            LmStudioTarget {
                url: DEFAULT_LEGION_LMSTUDIO_URL.to_string(),
                model: configured_model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_LEGION_LMSTUDIO_MODEL.to_string()),
            },
            LmStudioTarget {
                url: DEFAULT_AOSTAR_LMSTUDIO_URL.to_string(),
                model: configured_model.unwrap_or_else(|| DEFAULT_LMSTUDIO_MODEL.to_string()),
            },
        ];
    }

    vec![LmStudioTarget {
        url: DEFAULT_AOSTAR_LMSTUDIO_URL.to_string(),
        model: configured_model.unwrap_or_else(|| DEFAULT_LMSTUDIO_MODEL.to_string()),
    }]
}

fn configured_lmstudio_model() -> Option<String> {
    env::var("LMSTUDIO_EMBED_MODEL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| env::var("LMSTUDIO_MODEL").ok().filter(|v| !v.is_empty()))
}

fn http_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| value.starts_with("http"))
}

fn is_legion_machine() -> bool {
    ["COMPUTERNAME", "HOSTNAME"].iter().any(|key| {
        env::var(key)
            .ok()
            .map(|value| value.to_ascii_lowercase().contains("legion"))
            .unwrap_or(false)
    })
}
