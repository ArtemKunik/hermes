// tools/hermes-engine/src/arch_rules/classifier.rs
// TRACK-045: Classify a file path into an architectural layer.

/// The architectural layer a file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    Handler,
    Service,
    Store,
    Component,
    Hook,
    Api,
    Type,
    Test,
    Unknown,
}

impl Layer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Service => "service",
            Self::Store => "store",
            Self::Component => "component",
            Self::Hook => "hook",
            Self::Api => "api",
            Self::Type => "type",
            Self::Test => "test",
            Self::Unknown => "unknown",
        }
    }
}

/// Classify a file path (relative or absolute) into its architectural layer.
///
/// Matching order matters — more specific patterns take precedence.
pub fn classify_file(file_path: &str) -> Layer {
    let normalized = file_path.replace('\\', "/").to_lowercase();

    // Test files — must be checked before other layers
    if normalized.contains("/tests/")
        || normalized.ends_with("_test.rs")
        || normalized.ends_with(".test.tsx")
        || normalized.ends_with(".test.ts")
        || normalized.ends_with(".spec.ts")
        || normalized.ends_with(".spec.tsx")
    {
        return Layer::Test;
    }

    // Hooks — React custom hooks (data-fetching)
    if normalized.contains("/hooks/") {
        return Layer::Hook;
    }

    // API service clients — checked before generic Service
    if (normalized.contains("/services/") && normalized.contains("api"))
        || normalized.contains("/api_client")
        || normalized.contains("/http_client")
    {
        return Layer::Api;
    }

    // Handlers — thin HTTP/command handlers
    if normalized.contains("/handlers/") || normalized.contains("/handler/") {
        return Layer::Handler;
    }

    // Services — business logic orchestration
    if normalized.contains("_service/")
        || normalized.contains("/service/")
        || normalized.ends_with("_service.rs")
        || normalized.ends_with("service.ts")
    {
        return Layer::Service;
    }

    // Stores — database access layer
    if normalized.contains("store_")
        || normalized.contains("/store/")
        || normalized.contains("cosmos_")
        || normalized.ends_with("_store.rs")
        || normalized.ends_with("store.ts")
    {
        return Layer::Store;
    }

    // React components
    if normalized.contains("/components/") {
        return Layer::Component;
    }

    // Type definitions
    if normalized.contains("/types/")
        || normalized.ends_with("_types.rs")
        || normalized.ends_with(".types.ts")
        || normalized.ends_with("types.ts")
    {
        return Layer::Type;
    }

    Layer::Unknown
}

/// Return the naming convention description for a given layer.
pub fn naming_convention(layer: &Layer) -> &'static str {
    match layer {
        Layer::Handler => "snake_case for route functions; keep thin (no business logic)",
        Layer::Service => "snake_case for functions; PascalCase for structs/traits",
        Layer::Store => "snake_case for query functions; PascalCase for repository structs",
        Layer::Component => "PascalCase for component names; props interface as ComponentNameProps",
        Layer::Hook => "camelCase starting with 'use' (e.g. useTaskList)",
        Layer::Api => "camelCase for API client methods; BASE_URL constant for endpoint root",
        Layer::Type => "PascalCase for type aliases and interfaces",
        Layer::Test => "test_<function>_<condition>_<expected> for Rust; describe/it for TS",
        Layer::Unknown => "follow nearest matching convention",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_handler() {
        assert_eq!(classify_file("src/handlers/task_handler.rs"), Layer::Handler);
    }

    #[test]
    fn classify_store() {
        assert_eq!(classify_file("src/store_cosmos/tasks.rs"), Layer::Store);
        assert_eq!(classify_file("src/cosmos_store.rs"), Layer::Store);
    }

    #[test]
    fn classify_service() {
        assert_eq!(classify_file("src/task_service/mod.rs"), Layer::Service);
    }

    #[test]
    fn classify_component() {
        assert_eq!(classify_file("src/components/TaskList.tsx"), Layer::Component);
    }

    #[test]
    fn classify_hook() {
        assert_eq!(classify_file("src/hooks/useTaskList.ts"), Layer::Hook);
    }

    #[test]
    fn classify_test_rs() {
        assert_eq!(classify_file("src/task_service_test.rs"), Layer::Test);
    }

    #[test]
    fn classify_test_tsx() {
        assert_eq!(classify_file("src/TaskList.test.tsx"), Layer::Test);
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(classify_file("src/main.rs"), Layer::Unknown);
    }
}
