pub mod arch;
pub mod core;
pub mod memory;
pub mod missions;
pub mod proposals;
pub mod quality;
pub mod slow_loop;

use serde_json::{json, Value};

pub fn tools_list() -> Value {
    let mut tools = Vec::new();
    tools.extend(core::tools());
    tools.extend(memory::tools());
    tools.extend(missions::tools());
    tools.extend(proposals::tools());
    tools.extend(slow_loop::tools());
    tools.extend(quality::tools());
    tools.extend(arch::tools());

    json!({ "tools": tools })
}

pub fn tool_name_list() -> Vec<String> {
    tools_list()["tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
