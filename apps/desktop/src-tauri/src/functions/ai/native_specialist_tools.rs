//! Host-owned specialist delegation and persistence. UI transports only present progress and review.

use super::{
    copilot,
    frontend_tool_bridge::FrontendToolContext,
    native_platform_tools::{NativePlatformSession, segment},
    native_tool_bridge::run_options,
};
use crate::state::TauriFlowLikeState;
use dashmap::DashMap;
use flow_like::{
    a2ui::SurfaceComponent,
    copilot::{CopilotScope, UnifiedCopilotResponse},
    flow::board::Board,
};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock},
};
use tauri::{AppHandle, Manager, ipc::Channel};
use tokio::sync::{Mutex, OwnedMutexGuard};

pub(crate) mod board;
mod board_inspection;
mod home;
mod home_references;
pub use home::{
    __cmd__flowpilot_capture_home_snapshot, __cmd__flowpilot_get_home_proposal,
    __cmd__flowpilot_resolve_home_proposal, __tauri_command_name_flowpilot_capture_home_snapshot,
    __tauri_command_name_flowpilot_get_home_proposal,
    __tauri_command_name_flowpilot_resolve_home_proposal, flowpilot_capture_home_snapshot,
    flowpilot_get_home_proposal, flowpilot_resolve_home_proposal,
};
mod home_validation;
mod source;
pub(crate) mod widget;
mod widget_inspection;
mod widget_surface;
pub use widget_surface::{
    __cmd__flowpilot_capture_widget_snapshot, __cmd__flowpilot_get_widget_proposal,
    __cmd__flowpilot_resolve_widget_proposal,
    __tauri_command_name_flowpilot_capture_widget_snapshot,
    __tauri_command_name_flowpilot_get_widget_proposal,
    __tauri_command_name_flowpilot_resolve_widget_proposal, flowpilot_capture_widget_snapshot,
    flowpilot_get_widget_proposal, flowpilot_resolve_widget_proposal,
};

static TARGET_LOCKS: LazyLock<DashMap<String, Arc<Mutex<()>>>> = LazyLock::new(DashMap::new);

fn snapshot_owner(session: &NativePlatformSession) -> Value {
    let hub = reqwest::Url::parse(&session.hub)
        .map(|url| url.as_str().trim_end_matches('/').to_owned())
        .unwrap_or_else(|_| session.hub.clone());
    json!({"profileId":session.profile.id,"hub":hub,"subject":session.subject,
        "issuer":session.auth.as_ref().map(|binding| &binding.issuer)})
}

fn validate_snapshot_owner(
    session: &NativePlatformSession,
    expected: Option<&Value>,
) -> Result<(), String> {
    if expected != Some(&snapshot_owner(session)) {
        return Err(
            "The editor's profile or account changed before its draft was captured.".into(),
        );
    }
    Ok(())
}

pub(super) fn supports(tool: &str) -> bool {
    source::supports(tool)
        || home::supports(tool)
        || matches!(
            tool,
            "flowpilot_board"
                | "flowpilot_board_review"
                | "flowpilot_widget"
                | "data_studio_agent"
                | "research_agent"
                | "project_scout"
                | "flowpilot_home"
        )
}

/// Resolve review approval from the retained host artifact, never from model-provided summaries.
pub(super) fn approval(
    tool: &str,
    args: &Value,
) -> Result<Option<super::frontend_tool_bridge::FrontendToolApproval>, String> {
    if tool != "flowpilot_board_review"
        || !matches!(string(args, "action", "action"), Some("apply" | "dismiss"))
    {
        return Ok(None);
    }
    let job = copilot::flowpilot_get_board_edit_job(required(args, "job_id")?.to_owned())
        .ok_or("The board review is no longer retained.")?;
    if Some(job.app_id.as_str()) != string(args, "app_id", "appId")
        || Some(job.board_id.as_str()) != string(args, "board_id", "boardId")
    {
        return Err("The retained review belongs to another app or board.".into());
    }
    let spec = flow_like::flow::copilot::tool_spec::find_global_tool_spec(tool)
        .ok_or("The review approval policy is unavailable.")?;
    let resolved = flow_like::flow::copilot::tool_spec::resolve_tool_apply_approval(&spec, args);
    let description = format!(
        "{}\nExact retained review {}: {} command(s).\n{}\nDestructive effects: {}",
        resolved.description,
        job.job_id,
        job.review.command_count,
        job.review.command_summaries.join("\n"),
        job.review.destructive_effects.join("; ")
    );
    Ok(Some(super::frontend_tool_bridge::FrontendToolApproval {
        kind: resolved.kind,
        title: resolved.title,
        description,
        session_key: format!("{}:{}", resolved.session_key, job.token.claim_id),
    }))
}

pub(crate) fn execute<'a>(
    app: &'a AppHandle,
    tool: &'a str,
    args: &'a Value,
    context: Option<&'a FrontendToolContext>,
    request_id: &'a str,
) -> Pin<Box<dyn Future<Output = Option<Result<Value, String>>> + Send + 'a>> {
    Box::pin(async move {
        if !supports(tool) {
            return None;
        }
        Some(execute_inner(app, tool, args, context, request_id).await)
    })
}

async fn execute_inner(
    app: &AppHandle,
    tool: &str,
    args: &Value,
    context: Option<&FrontendToolContext>,
    request_id: &str,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(app, context).await?;
    if source::supports(tool) {
        return source::execute(&session, tool, args).await;
    }
    if home::supports(tool) {
        return home::execute(&session, tool, args).await;
    }
    match tool {
        "flowpilot_board" => board::execute(&session, args, request_id).await,
        "flowpilot_board_review" => board::review(&session, args).await,
        "flowpilot_widget" => widget::execute(&session, args, request_id).await,
        _ => {
            let (scope, instruction, read_only) = match tool {
                "data_studio_agent" => (
                    CopilotScope::DataStudio,
                    required(args, "instruction")?.to_owned(),
                    false,
                ),
                "project_scout" => (CopilotScope::Scout, scout_instruction(args)?, true),
                // The model may have read private workspace content. Public research receives
                // only the original user request, never model-authored arguments or app context.
                "research_agent" => (
                    CopilotScope::Research,
                    context
                        .and_then(|c| c.source_user_prompt.clone())
                        .filter(|s| !s.trim().is_empty())
                        .ok_or(
                            "The immutable user request is unavailable for sealed public research.",
                        )?,
                    true,
                ),
                "flowpilot_home" => (
                    CopilotScope::Home,
                    required(args, "instruction")?.to_owned(),
                    false,
                ),
                _ => unreachable!(),
            };
            let attachments = if scope == CopilotScope::DataStudio {
                super::native_platform_tools::select_forwarded_attachments(
                    &run_options(context).attachments,
                    args.get("forward_files"),
                )?
            } else {
                Vec::new()
            };
            let instruction = if attachments.is_empty() {
                instruction
            } else {
                format!(
                    "{instruction}\n\nFILES FORWARDED BY THE USER\n{}",
                    attachments
                        .iter()
                        .map(|file| format!(
                            "- {} ({}, {} bytes)",
                            file.name.as_deref().unwrap_or("unnamed"),
                            file.mime_type
                                .as_deref()
                                .unwrap_or("application/octet-stream"),
                            file.size.unwrap_or(0)
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            let app_id = if matches!(scope, CopilotScope::DataStudio) {
                let id = session.app_id(args)?;
                session.require_visible(&id)?;
                Some(id)
            } else {
                None
            };
            let mut nested = nested_context(&session, request_id, app_id.clone(), None);
            nested.overlay_id = string(args, "overlay_id", "overlayId").map(str::to_owned);
            if scope == CopilotScope::Research {
                nested.app_id = None;
                nested.board_id = None;
                nested.overlay_id = None;
                nested.board_context_manifest = None;
            }
            let response = run_specialist(
                &session,
                scope,
                instruction,
                read_only,
                None,
                None,
                None,
                nested,
                request_id,
                attachments,
            )
            .await?;
            if scope == CopilotScope::Home {
                if let Some(mut receipt) = home::staged_receipt(request_id) {
                    receipt["message"] = json!(response.message);
                    return Ok(receipt);
                }
            }
            Ok(
                json!({"status":"ok", "message":response.message, "app_id":app_id, "runtime_verified":false}),
            )
        }
    }
}

fn scout_instruction(args: &Value) -> Result<String, String> {
    let mut instruction = format!("Research goal: {}", required(args, "goal")?);
    for (field, label) in [
        ("focus", "Focus"),
        ("app_id", "Starting app"),
        ("template_id", "Starting template"),
    ] {
        if let Some(value) = string(args, field, field) {
            instruction.push_str(&format!("\n{label}: {value}"));
        }
    }
    if let Some(candidates) = args.get("candidates").and_then(Value::as_array) {
        let ids = candidates
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            instruction.push_str(&format!(
                "\nRestrict the search to these apps: {}.",
                ids.join(", ")
            ));
        }
    }
    Ok(instruction)
}

pub(crate) fn nested_context(
    session: &NativePlatformSession,
    request_id: &str,
    app_id: Option<String>,
    board_id: Option<String>,
) -> FrontendToolContext {
    let mut context = session.context.clone().unwrap_or_default();
    context.profile_id = Some(session.profile.id.clone());
    context.parent_request_id = Some(request_id.to_owned());
    context.app_id = app_id;
    context.board_id = board_id;
    context
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_specialist(
    session: &NativePlatformSession,
    scope: CopilotScope,
    instruction: String,
    read_only: bool,
    board: Option<Board>,
    components: Option<Vec<SurfaceComponent>>,
    canvas: Option<Value>,
    context: FrontendToolContext,
    request_id: &str,
    attachments: Vec<flow_like::flow::copilot::AttachmentManifestEntry>,
) -> Result<UnifiedCopilotResponse, String> {
    run_specialist_with_selection(
        session,
        scope,
        instruction,
        read_only,
        board,
        components,
        canvas,
        None,
        context,
        request_id,
        attachments,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn run_specialist_with_selection(
    session: &NativePlatformSession,
    scope: CopilotScope,
    instruction: String,
    read_only: bool,
    board: Option<Board>,
    components: Option<Vec<SurfaceComponent>>,
    canvas: Option<Value>,
    selection: Option<Vec<String>>,
    mut context: FrontendToolContext,
    request_id: &str,
    attachments: Vec<flow_like::flow::copilot::AttachmentManifestEntry>,
) -> Result<UnifiedCopilotResponse, String> {
    session.assert_current_profile().await?;
    let options = run_options(session.context.as_ref());
    let child_id = format!("{request_id}:agent");
    let _child = super::native_tool_bridge::claim_child_run(&context, &child_id, attachments)?;
    context.run_id = Some(child_id.clone());
    let token = if let Some(binding) = &session.auth {
        Some(binding.cached_access_token().await?)
    } else {
        None
    };
    let app_id = context.app_id.clone();
    let raw = context
        .source_user_prompt
        .clone()
        .unwrap_or_else(|| instruction.clone());
    let tool_name = match scope {
        CopilotScope::Board => "flowpilot_board",
        CopilotScope::Frontend => "flowpilot_widget",
        CopilotScope::DataStudio => "data_studio_agent",
        CopilotScope::Research => "research_agent",
        CopilotScope::Scout => "project_scout",
        CopilotScope::Home => "flowpilot_home",
        _ => "specialist",
    };
    let emit_lifecycle = |stage: &str, status: &str, error: Option<&str>| {
        super::native_tool_bridge::emit_run_frame(session.context.as_ref(),json!({"type":"native_specialist_event","requestId":child_id,"parentRequestId":request_id,"toolName":tool_name,"stage":stage,"status":status,"error":error,"frame":""}).to_string())
    };
    emit_lifecycle("started", "progress", None);
    let parent_context = session.context.clone();
    let stream_child = child_id.clone();
    let stream_parent = request_id.to_owned();
    let channel = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
        if let tauri::ipc::InvokeResponseBody::Json(json) = body {
            if let Ok(frame) = serde_json::from_str::<String>(&json) {
                super::native_tool_bridge::emit_run_frame(parent_context.as_ref(),json!({"type":"native_specialist_event","requestId":stream_child,"parentRequestId":stream_parent,"toolName":tool_name,"frame":frame}).to_string());
            }
        }
        Ok(())
    });
    let response = copilot::copilot_chat_native(
        session.app_handle.clone(),
        None,
        session.app_handle.state::<TauriFlowLikeState>(),
        scope,
        board,
        None,
        None,
        components,
        canvas,
        selection,
        instruction,
        None,
        None,
        options.model_id,
        options.reasoning_effort,
        token,
        None,
        None,
        Some(true),
        Some(read_only),
        app_id,
        Some(context),
        Some(format!("{request_id}:agent")),
        Some(raw),
        channel,
    )
    .await;
    emit_lifecycle(
        "finished",
        if response.is_ok() { "ok" } else { "error" },
        response.as_ref().err().map(String::as_str),
    );
    let response = response?;
    session.assert_current_profile().await?;
    Ok(response)
}

async fn target_lock(key: String) -> OwnedMutexGuard<()> {
    // Drop the DashMap guard before waiting, so another target on the same shard can progress.
    if TARGET_LOCKS.len() > 256 {
        TARGET_LOCKS.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
    let lock = TARGET_LOCKS
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    lock.lock_owned().await
}

fn specialist_app_id(session: &NativePlatformSession, args: &Value) -> Result<String, String> {
    if let Some(created) =
        super::native_platform_tools::created_app_for_run(session.context.as_ref())
    {
        if string(args, "app_id", "appId").is_some_and(|id| id != created) {
            return Err(
                "This run created a new app. Delegate changes to that returned app_id.".into(),
            );
        }
        return Ok(created);
    }
    session.app_id(args)
}

fn string<'a>(args: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}
fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    string(args, key, key).ok_or_else(|| format!("This operation requires {key}."))
}
fn stable_artifact_id(
    session: &NativePlatformSession,
    request_id: &str,
    kind: &str,
    args: &Value,
) -> String {
    let scope = session
        .context
        .as_ref()
        .and_then(|c| c.conversation_id.as_deref())
        .unwrap_or(request_id);
    let retry = string(args, "idempotency_key", "idempotencyKey")
        .or_else(|| string(args, "instruction", "instruction"))
        .unwrap_or(request_id);
    let identity = json!([
        session.profile.id,
        scope,
        kind,
        args.get("app_id"),
        args.get("board_id"),
        retry
    ]);
    format!(
        "flowpilot-{}",
        &blake3::hash(identity.to_string().as_bytes()).to_hex()[..24]
    )
}

pub(crate) fn contract_fingerprint(namespace: &str, value: &Value) -> String {
    home::fingerprint_for(namespace, value)
}

pub(crate) fn contract_key_order(left: &str, right: &str) -> std::cmp::Ordering {
    home::compare_keys(left, right)
}
