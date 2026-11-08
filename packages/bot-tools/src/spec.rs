use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    None,
    Ask,
}

/// What core registers for a tool: the model-facing description, the JSON Schema of its
/// arguments and the gates.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// The capability that switches the tool on; `None` for tools every bot with a workspace has.
    pub capability: Option<&'static str>,
    pub module: &'static str,
    pub approval: Approval,
    /// Arguments that make a single call ask for approval when set (dotted paths).
    pub approval_args: &'static [&'static str],
    pub mutating: bool,
    pub input_schema: Value,
}

pub fn schema_of<T: JsonSchema>() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
        if object.get("type").is_some_and(|kind| kind == "object") {
            object
                .entry("properties")
                .or_insert_with(|| Value::Object(Default::default()));
        }
    }
    schema
}

pub fn specs() -> Vec<ToolSpec> {
    let mut all = crate::files::specs();
    all.extend(crate::configuration::specs());
    all.extend(crate::knowledge::specs());
    all.extend(crate::documents::specs());
    all.extend(crate::code::specs());
    all.extend(crate::image::specs());
    all.extend(crate::email::specs());
    all.extend(crate::notify::specs());
    all.extend(crate::people::specs());
    all.extend(crate::user_context::specs());
    all.extend(crate::schedule::specs());
    all.extend(crate::skills::specs());
    all.extend(crate::browser::specs());
    all.extend(crate::computer::specs());
    all.extend(crate::shell::specs());
    all.extend(crate::secrets::specs());
    all.extend(crate::services::specs());
    all.extend(crate::web::specs());
    #[cfg(feature = "data")]
    all.extend(crate::data::specs());
    all
}

pub fn spec(name: &str) -> Option<ToolSpec> {
    specs().into_iter().find(|spec| spec.name == name)
}
