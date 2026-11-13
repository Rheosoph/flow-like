//! Plain FlowPilot (no bot) runs the bot document tools on the files attached to its chat. The
//! tools see each attachment as a conversation-file handle; this host reads it from the chat's
//! upload and builds FlowPilot's own model to read it with.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::{
    flow::copilot::{
        AttachmentManifestEntry, resolve_host_completion_model, tool_spec::FLOWPILOT_FILE_TOOLS,
    },
    flow_like_model_provider::llm::ModelLogic,
    flow_like_storage::{
        contracts::column_metadata::ColumnReader, files::store::FlowLikeStore, lancedb::Connection,
    },
    models::llm::ModelUsageContext,
};
use flow_like_bot_tools::{
    FileData, KnowledgeEmbedder, MemorySessions, Scope, SecretInfo, documents::FILE_EXTRACT,
};
use flow_like_types::{Result, anyhow, data_classification::DataClassification};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::super::{
    copilot::{bits_model_id, chat_file_handle, read_chat_file},
    frontend_tool_bridge::FrontendToolContext,
    native_bot_tools::{DesktopToolEnv, ToolHost, run_tool},
    native_platform_tools::{NativePlatformSession, select_forwarded_attachments},
    native_tool_bridge::{execution_cancellation, run_options},
};
use crate::state::TauriFlowLikeState;

pub(crate) fn runs(tool: &str) -> bool {
    FLOWPILOT_FILE_TOOLS.contains(&tool)
}

pub(crate) async fn execute(
    app: &AppHandle,
    context: Option<&FrontendToolContext>,
    tool: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let session = NativePlatformSession::new(app, context).await?;
    session.assert_current_profile().await?;
    let options = run_options(context);
    let args = with_attachment_handles(tool, args, &options.attachments)?;
    let host = Arc::new(FlowPilotHost {
        session,
        attachments: options.attachments.clone(),
        model_id: bits_model_id(options.model_id.clone()),
        run_id: context.and_then(|context| context.run_id.clone()),
    });
    let scope = Scope {
        workspace: None,
        apps: Vec::new(),
        storage: false,
        data_connections: false,
        classification: DataClassification::Internal,
    };
    let env = DesktopToolEnv::new(
        host.clone(),
        scope,
        Arc::new(MemorySessions::default()),
        execution_cancellation(),
        options.auto_approve,
    );
    let value = run_tool(env, tool, args).await?;
    host.session.assert_current_profile().await?;
    Ok(value)
}

/// Replaces the attachment names in `file` (or `files` for `file_extract`) with handles.
fn with_attachment_handles(
    tool: &str,
    args: &Value,
    attachments: &[AttachmentManifestEntry],
) -> std::result::Result<Value, String> {
    let mut args = args.clone();
    let field = if tool == FILE_EXTRACT { "files" } else { "file" };
    match args.get_mut(field) {
        Some(Value::Array(items)) => {
            for item in items.iter_mut() {
                *item = attachment_handle(item, attachments)?;
            }
        }
        Some(item) => *item = attachment_handle(item, attachments)?,
        None => {}
    }
    Ok(args)
}

fn attachment_handle(
    value: &Value,
    attachments: &[AttachmentManifestEntry],
) -> std::result::Result<Value, String> {
    let name = value
        .as_str()
        .ok_or("Name a file attached to this chat by its exact name.")?;
    let handle = select_forwarded_attachments(attachments, Some(&json!([name])))?
        .first()
        .and_then(chat_file_handle)
        .ok_or_else(|| format!("No file named '{name}' is attached to this chat."))?;
    serde_json::to_value(handle).map_err(|error| error.to_string())
}

struct FlowPilotHost {
    session: NativePlatformSession,
    attachments: Vec<AttachmentManifestEntry>,
    model_id: Option<String>,
    run_id: Option<String>,
}

#[async_trait]
impl ToolHost for FlowPilotHost {
    async fn store(&self, _app_id: &str) -> Result<FlowLikeStore> {
        Err(anyhow!("FlowPilot reads only the files attached to its chat"))
    }

    async fn tables(&self, _app_id: &str) -> Result<(Connection, ColumnReader)> {
        Err(anyhow!("FlowPilot has no tables to save extractions into"))
    }

    async fn model(&self) -> Result<Arc<dyn ModelLogic>> {
        let state = self.session.app_handle.state::<TauriFlowLikeState>();
        let mut model_state = (*state.0).clone();
        model_state.device_model_connector = None;
        let token = match &self.session.auth {
            Some(binding) => Some(
                crate::execution_credentials::authorize_native_models(&mut model_state, binding)
                    .await
                    .map_err(|error| anyhow!(error))?,
            ),
            None => None,
        };
        let model_state = Arc::new(model_state);
        let bit = resolve_host_completion_model(
            &model_state,
            &self.session.profile,
            self.model_id.as_deref(),
        )
        .await?;
        model_state
            .model_factory
            .build(
                &bit,
                model_state.clone(),
                token,
                Some(ModelUsageContext {
                    app_id: None,
                    run_id: self.run_id.clone(),
                    api_base_url: None,
                }),
            )
            .await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        Ok(None)
    }

    async fn secret(&self, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }

    async fn secrets(&self) -> Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }

    async fn conversation_file(
        &self,
        _conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        read_chat_file(&self.attachments, path, max_bytes)
            .await
            .map(Some)
    }
}
