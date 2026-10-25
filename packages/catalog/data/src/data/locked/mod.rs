use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Value, anyhow, json::json};

pub mod client;
pub mod download_item;
pub mod extend_retention;
pub mod get_item;
pub mod list_items;
pub mod purge;
pub mod set_auto_delete;
pub mod set_legal_hold;
pub mod storage_status;
pub mod transfer;
pub mod types;
pub mod upload_item;
pub mod upload_version;

use client::{LockedAction, LockedApi};
use transfer::{LockedUploadFields, LockedUploadResult};
use types::{
    GetLockedItemRequest, LockedFile, LockedItemDetail, LockedItemVersion, LockedMetadata,
    LockedScope,
};

const SCOPES: [&str; 2] = ["project", "user"];

pub(crate) fn locked_node(
    name: &str,
    label: &str,
    description: &str,
    alias: &str,
    performance: u8,
) -> Node {
    let mut node = Node::new(name, label, description, "Data/Files/Locked");
    node.set_version(2);
    node.set_flowscript_name("lockedFiles", alias);
    node.set_receiver("");
    node.add_icon("/flow/icons/lock.svg");
    node.set_scores(
        NodeScores::new()
            .set_privacy(5)
            .set_security(3)
            .set_performance(performance)
            .set_governance(2)
            .set_reliability(3)
            .set_cost(4)
            .build(),
    );
    node.add_input_pin(
        "exec_in",
        "In",
        "Start the request",
        VariableType::Execution,
    );
    node
}

pub(crate) fn add_exec_out(node: &mut Node, description: &str) {
    node.add_output_pin("exec_out", "Out", description, VariableType::Execution);
}

pub(crate) fn add_text_pin(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_input_pin(name, label, description, VariableType::String)
        .set_default_value(Some(json!("")));
}

pub(crate) fn add_scope_pin(node: &mut Node) {
    node.add_input_pin(
        "scope",
        "Scope",
        "project: shared with everyone in the project; user: private to the user running the flow",
        VariableType::String,
    )
    .set_default_value(Some(json!("project")))
    .set_options(
        PinOptions::new()
            .set_valid_values(SCOPES.iter().map(|scope| scope.to_string()).collect())
            .build(),
    );
}

pub(crate) fn add_source_pin(node: &mut Node) {
    node.add_input_pin(
        "path",
        "File",
        "File to lock; it is read twice, once to hash and once to upload",
        VariableType::Struct,
    )
    .set_schema::<FlowPath>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

pub(crate) fn add_folder_pin(node: &mut Node, description: &str) {
    add_text_pin(node, "folder", "Folder", description);
}

pub(crate) fn add_item_id_pin(node: &mut Node, description: &str) {
    add_text_pin(node, "item_id", "Locked File ID", description);
}

pub(crate) fn add_upload_option_pins(node: &mut Node) {
    node.add_input_pin(
        "retention_days",
        "Retention Days",
        "Days the version stays write-once and undeletable; 0 uses the server default. The server clamps it to its allowed range and ignores it under a bucket-wide policy",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
    node.add_input_pin(
        "delete_after_retention",
        "Delete After Retention",
        "Delete the version automatically once its retention ends and no legal hold remains; otherwise it stays stored until someone deletes it. Project files need app admin rights",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
    add_text_pin(
        node,
        "comment",
        "Comment",
        "Note stored with this version, such as why it was archived",
    );
    node.add_input_pin(
        "metadata",
        "Metadata",
        "Optional JSON object stored with this version",
        VariableType::Struct,
    )
    .set_open_schema()
    .set_default_value(Some(json!({})));
}

pub(crate) fn add_file_output(node: &mut Node, description: &str) {
    node.add_output_pin("file", "Locked File", description, VariableType::Struct)
        .set_schema::<LockedFile>();
}

pub(crate) fn add_versions_output(node: &mut Node) {
    node.add_output_pin(
        "versions",
        "Versions",
        "Every version of the file, newest first, including purged ones",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<LockedItemVersion>();
}

pub(crate) fn add_version_output(node: &mut Node, description: &str) {
    node.add_output_pin("version", "Version", description, VariableType::Struct)
        .set_schema::<LockedItemVersion>();
}

pub(crate) fn optional_text(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) fn normalize_folder(folder: &str) -> String {
    folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn parse_scope(scope: &str) -> flow_like_types::Result<LockedScope> {
    match scope.trim().to_ascii_lowercase().as_str() {
        "" | "project" => Ok(LockedScope::Project),
        "user" => Ok(LockedScope::User),
        other => Err(anyhow!(
            "Unknown locked storage scope '{other}'; use project or user"
        )),
    }
}

pub(crate) fn zero_as_none(label: &str, days: i64) -> flow_like_types::Result<Option<u32>> {
    match days {
        0 => Ok(None),
        days if days < 0 => Err(anyhow!("{label} must not be negative, got {days}")),
        days => u32::try_from(days)
            .map(Some)
            .map_err(|_| anyhow!("{label} is too large: {days}")),
    }
}

pub(crate) fn metadata_object(value: Value) -> flow_like_types::Result<Option<LockedMetadata>> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) if map.is_empty() => Ok(None),
        Value::Object(map) => Ok(Some(map)),
        other => Err(anyhow!(
            "Locked file metadata must be a JSON object, got {other}"
        )),
    }
}

pub(crate) async fn read_scope(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<LockedScope> {
    parse_scope(&context.evaluate_pin::<String>("scope").await?)
}

pub(crate) async fn read_required(
    context: &mut ExecutionContext,
    pin: &str,
    label: &str,
) -> flow_like_types::Result<String> {
    optional_text(context.evaluate_pin::<String>(pin).await?)
        .ok_or_else(|| anyhow!("{label} is required"))
}

pub(crate) async fn read_upload_options(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<LockedUploadFields> {
    Ok(LockedUploadFields {
        retention_days: zero_as_none(
            "Retention Days",
            context.evaluate_pin::<i64>("retention_days").await?,
        )?,
        delete_after_retention: context
            .evaluate_pin::<bool>("delete_after_retention")
            .await?,
        comment: optional_text(context.evaluate_pin::<String>("comment").await?),
        metadata: metadata_object(context.evaluate_pin::<Value>("metadata").await?)?,
        ..Default::default()
    })
}

pub(crate) async fn get_detail(
    context: &ExecutionContext,
    api: &LockedApi,
    item_id: &str,
) -> flow_like_types::Result<LockedItemDetail> {
    api.find(
        context,
        LockedAction::Get,
        &GetLockedItemRequest {
            item_id: Some(item_id),
            ..Default::default()
        },
    )
    .await?
    .ok_or_else(|| anyhow!("No locked file with ID '{item_id}' exists in this app"))
}

/// The given version ID, else the newest locked version of the file.
pub(crate) async fn resolve_version_id(
    context: &ExecutionContext,
    api: &LockedApi,
    item_id: &str,
    version_id: Option<String>,
) -> flow_like_types::Result<String> {
    if let Some(version_id) = version_id {
        return Ok(version_id);
    }
    let detail = get_detail(context, api, item_id).await?;
    detail
        .latest_locked()
        .map(|version| version.id.clone())
        .ok_or_else(|| anyhow!("Locked file '{item_id}' has no locked version"))
}

pub(crate) async fn set_detail_outputs(
    context: &mut ExecutionContext,
    detail: &LockedItemDetail,
) -> flow_like_types::Result<()> {
    context
        .set_pin_value("file", json!(LockedFile::from(detail)))
        .await?;
    context
        .set_pin_value("versions", json!(detail.versions))
        .await
}

pub(crate) async fn set_upload_outputs(
    context: &mut ExecutionContext,
    upload: &LockedUploadResult,
) -> flow_like_types::Result<()> {
    let version = upload
        .detail
        .version_by_id(&upload.version_id)
        .ok_or_else(|| {
            anyhow!(
                "The server committed the upload but did not return version '{}'",
                upload.version_id
            )
        })?;
    context
        .set_pin_value(
            "file",
            json!(LockedFile::new(&upload.detail.item, Some(version))),
        )
        .await?;
    context.set_pin_value("version", json!(version)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_lose_empty_segments_and_outer_slashes() {
        assert_eq!(normalize_folder(""), "");
        assert_eq!(normalize_folder("/"), "");
        assert_eq!(normalize_folder("/invoices/2026/"), "invoices/2026");
        assert_eq!(normalize_folder(" a // b \\ c "), "a/b/c");
    }

    #[test]
    fn scopes_are_project_or_user() {
        assert_eq!(parse_scope("").unwrap(), LockedScope::Project);
        assert_eq!(parse_scope(" Project ").unwrap(), LockedScope::Project);
        assert_eq!(parse_scope("user").unwrap(), LockedScope::User);
        assert!(parse_scope("team").is_err());
    }

    #[test]
    fn zero_days_means_the_server_default() {
        assert_eq!(zero_as_none("Retention Days", 0).unwrap(), None);
        assert_eq!(zero_as_none("Retention Days", 30).unwrap(), Some(30));
        assert!(zero_as_none("Retention Days", -1).is_err());
        assert!(zero_as_none("Retention Days", i64::MAX).is_err());
    }

    #[test]
    fn metadata_is_an_optional_object() {
        assert_eq!(metadata_object(Value::Null).unwrap(), None);
        assert_eq!(metadata_object(json!({})).unwrap(), None);
        assert_eq!(
            metadata_object(json!({ "a": 1 })).unwrap().unwrap()["a"],
            json!(1)
        );
        assert!(metadata_object(json!([1])).is_err());
    }
}
