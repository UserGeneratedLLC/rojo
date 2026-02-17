use rbx_dom_weak::{types::Variant, InstanceBuilder, WeakDom};
use std::path::Path;

/// Write a WeakDom to an rbxm file at the given path.
/// The dom's root is serialized as the single top-level instance.
/// For syncback, `process_model_dom` in syncback.rs will take this
/// root instance as the project tree root.
pub fn write_rbxm(path: &Path, dom: &WeakDom) {
    let parent = path.parent().unwrap();
    if !parent.exists() {
        std::fs::create_dir_all(parent).expect("Failed to create fixture directory");
    }

    let mut buffer = Vec::new();
    rbx_binary::to_writer(&mut buffer, dom, &[dom.root_ref()])
        .expect("Failed to serialize rbxm fixture");
    std::fs::write(path, buffer).expect("Failed to write rbxm fixture");
}

/// Build a Folder instance.
pub fn folder(name: &str) -> InstanceBuilder {
    InstanceBuilder::new("Folder").with_name(name)
}

/// Build a Script instance with the given Source.
pub fn server_script(name: &str, source: &str) -> InstanceBuilder {
    InstanceBuilder::new("Script")
        .with_name(name)
        .with_property("Source", Variant::String(source.to_string()))
}

/// Build a ModuleScript instance with the given Source.
pub fn module_script(name: &str, source: &str) -> InstanceBuilder {
    InstanceBuilder::new("ModuleScript")
        .with_name(name)
        .with_property("Source", Variant::String(source.to_string()))
}

/// Build an ObjectValue instance (for Ref testing).
pub fn object_value(name: &str) -> InstanceBuilder {
    InstanceBuilder::new("ObjectValue").with_name(name)
}

/// Build a Part instance.
pub fn part(name: &str) -> InstanceBuilder {
    InstanceBuilder::new("Part").with_name(name)
}

/// Build a Model instance.
pub fn model(name: &str) -> InstanceBuilder {
    InstanceBuilder::new("Model").with_name(name)
}

/// Create the standard project json5 content for syncback tests.
pub fn standard_project_json5(name: &str) -> String {
    format!(
        r#"{{
  "name": "{name}",
  "tree": {{
    "$path": "src"
  }}
}}"#
    )
}

/// Ensure a directory and its parents exist.
pub fn ensure_dir(path: &Path) {
    if !path.exists() {
        std::fs::create_dir_all(path).expect("Failed to create directory");
    }
}
