use super::{
    add_exec_out, add_file_output, add_folder_pin, add_scope_pin, add_source_pin, add_text_pin,
    add_upload_option_pins, add_version_output,
    client::{LockedAccess, LockedApi},
    locked_node, normalize_folder, optional_text, read_scope, read_upload_options,
    set_upload_outputs,
    transfer::upload_locked,
    types::LockedIfExists,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{anyhow, async_trait, json::json};

const IF_EXISTS: [&str; 2] = ["error", "new_version"];

fn parse_if_exists(value: &str) -> flow_like_types::Result<LockedIfExists> {
    match value.trim() {
        "" | "error" => Ok(LockedIfExists::Error),
        "new_version" => Ok(LockedIfExists::NewVersion),
        other => Err(anyhow!(
            "Unknown If Exists choice '{other}'; use error or new_version"
        )),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LockedUploadItemNode;

impl LockedUploadItemNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedUploadItemNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_upload_item",
            "Upload Locked File",
            "Stores a file write-once and revision-safe: once committed nobody can change or delete it until its retention ends. Fails when storage is full",
            "upload",
            6,
        );
        node.set_long_running(true);
        add_source_pin(&mut node);
        add_scope_pin(&mut node);
        add_folder_pin(
            &mut node,
            "Folder to store the file in, such as invoices/2026; empty for the root",
        );
        add_text_pin(
            &mut node,
            "name",
            "Name",
            "Display name of the locked file; empty uses the file's name",
        );
        add_upload_option_pins(&mut node);
        node.add_input_pin(
            "if_exists",
            "If Exists",
            "error: fail when a file with this name already exists in the folder; new_version: add the upload as its next version",
            VariableType::String,
        )
        .set_default_value(Some(json!("error")))
        .set_options(
            PinOptions::new()
                .set_valid_values(IF_EXISTS.iter().map(|value| value.to_string()).collect())
                .build(),
        );

        add_exec_out(&mut node, "The file was uploaded, verified and locked");
        add_file_output(&mut node, "The locked file with the uploaded version");
        add_version_output(&mut node, "The uploaded version with its lock details");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path = context.evaluate_pin::<FlowPath>("path").await?;
        let mut fields = read_upload_options(context).await?;
        fields.scope = Some(read_scope(context).await?);
        fields.folder = Some(normalize_folder(
            &context.evaluate_pin::<String>("folder").await?,
        ));
        fields.name = optional_text(context.evaluate_pin::<String>("name").await?);
        fields.if_exists = Some(parse_if_exists(
            &context.evaluate_pin::<String>("if_exists").await?,
        )?);
        let api = LockedApi::bind(context, LockedAccess::Write)?;
        let upload = upload_locked(context, &api, &path, fields).await?;
        set_upload_outputs(context, &upload).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_exists_defaults_to_error() {
        assert_eq!(parse_if_exists("").unwrap(), LockedIfExists::Error);
        assert_eq!(parse_if_exists("error").unwrap(), LockedIfExists::Error);
        assert_eq!(
            parse_if_exists(" new_version ").unwrap(),
            LockedIfExists::NewVersion
        );
        assert!(parse_if_exists("overwrite").is_err());
    }
}
