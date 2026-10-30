use super::*;
use crate::functions::{app, flow::board as board_commands};
use copilot::{BoardEditJob, BoardEditJobPhase};
use flow_like::{
    app::App,
    flow::ast::{RenderOptions, board_to_flowscript},
};
use reqwest::Method;

pub(crate) async fn load_app(session: &NativePlatformSession, app_id: &str) -> Result<App, String> {
    session.require_visible(app_id)?;
    if !session.is_local(app_id).await? {
        let value = session
            .request(Method::GET, &format!("apps/{}", segment(app_id)), None)
            .await?;
        let remote: App = serde_json::from_value(value).map_err(|e| e.to_string())?;
        app::update_app(session.app_handle.clone(), remote)
            .await
            .map_err(|e| e.to_string())?;
    }
    app::get_app(session.app_handle.clone(), app_id.to_owned())
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn load_board(
    session: &NativePlatformSession,
    app_id: &str,
    board_id: &str,
) -> Result<Board, String> {
    let app = load_app(session, app_id).await?;
    if !app.boards.iter().any(|id| id == board_id) {
        return Err("The board does not belong to the requested app.".into());
    }
    if !session.is_local(app_id).await? {
        let remote = session
            .request(
                Method::GET,
                &format!("apps/{}/board/{}", segment(app_id), segment(board_id)),
                None,
            )
            .await?;
        let board: Board = serde_json::from_value(remote).map_err(|e| e.to_string())?;
        if board.id != board_id {
            return Err("The hub returned another board.".into());
        }
        return Ok(board);
    }
    board_commands::get_board(
        session.app_handle.clone(),
        app_id.to_owned(),
        board_id.to_owned(),
        None,
    )
    .await
    .map(|b| (*b).clone())
    .map_err(|e| e.to_string())
}

pub(crate) async fn ensure_board(
    session: &NativePlatformSession,
    app_id: &str,
    board_id: &str,
    name: &str,
    description: &str,
) -> Result<(), String> {
    session.assert_current_profile().await?;
    if !session.is_local(app_id).await? {
        session.request(Method::PUT, &format!("apps/{}/board/{}", segment(app_id), segment(board_id)), Some(&json!({"name":name,"description":description,"log_level":"Debug","stage":"Dev"}))).await?;
        load_app(session, app_id).await?;
    }
    app::upsert_board(
        session.app_handle.clone(),
        app_id.to_owned(),
        Some(board_id.to_owned()),
        name.to_owned(),
        description.to_owned(),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
    request_id: &str,
) -> Result<Value, String> {
    let app_id = specialist_app_id(session, args)?;
    let mode = string(args, "mode", "mode").unwrap_or("edit");
    if !["edit", "explain", "inspect"].contains(&mode) {
        return Err("Unknown flowpilot_board mode.".into());
    }
    let requested_board = string(args, "board_id", "boardId").map(str::to_owned);
    if mode == "inspect" {
        let board_id = requested_board.ok_or("inspect requires board_id.")?;
        let lock = TARGET_LOCKS
            .entry(format!("{}:board:{app_id}:{board_id}", session.profile.id))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let Ok(_guard) = lock.try_lock_owned() else {
            return Ok(
                json!({"status":"in_progress","mode":"inspect","app_id":app_id,"board_id":board_id,"read_only":true,"persisted_readback_verified":false,"active_run":null,"message":"A board operation still owns this target. Wait for it to settle and inspect again."}),
            );
        };
        let board = load_board(session, &app_id, &board_id).await?;
        let result = inspection(&app_id, &board);
        let after = load_board(session, &app_id, &board_id).await?;
        if serde_json::to_value(&board).map_err(|e| e.to_string())?
            != serde_json::to_value(after).map_err(|e| e.to_string())?
        {
            return Ok(
                json!({"status":"error","code":"FLOWPILOT_BOARD_INSPECT_SNAPSHOT_CHANGED","message":"The board changed during inspection. Retry the exact target after its edit finishes."}),
            );
        }
        session.assert_current_profile().await?;
        return Ok(result);
    }
    let instruction = required(args, "instruction")?.to_owned();
    let creation_guard = target_lock(format!("{}:app:{app_id}", session.profile.id)).await;
    let app = load_app(session, &app_id).await?;
    let new_board = args
        .get("create_new_board")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let board_id = requested_board
        .or_else(|| (!new_board).then(|| app.boards.first().cloned()).flatten())
        .unwrap_or_else(|| stable_artifact_id(session, request_id, "board", args));
    let created = !app.boards.contains(&board_id);
    if created {
        ensure_board(
            session,
            &app_id,
            &board_id,
            string(args, "board_name", "boardName").unwrap_or("Main Board"),
            &instruction.chars().take(140).collect::<String>(),
        )
        .await?;
    }
    let _board_guard =
        target_lock(format!("{}:board:{app_id}:{board_id}", session.profile.id)).await;
    drop(creation_guard);
    let retained = copilot::flowpilot_list_board_edit_jobs(
        Some(app_id.clone()),
        Some(board_id.clone()),
        Some(false),
    );
    if let Some(job) = retained.iter().find(|job| {
        matches!(
            job.phase,
            BoardEditJobPhase::Preparing
                | BoardEditJobPhase::Failed
                | BoardEditJobPhase::AwaitingApproval
                | BoardEditJobPhase::Applying
                | BoardEditJobPhase::AppliedPendingDelivery
        )
    }) {
        return Ok(
            json!({"status":"awaiting_approval","app_id":app_id,"board_id":board_id,"job_id":job.job_id,"phase":job.phase,"applied":job.phase == BoardEditJobPhase::AppliedPendingDelivery,"message":"Resolve the existing retained board review before generating another edit."}),
        );
    }
    let board = load_board(session, &app_id, &board_id).await?;
    if !session.is_local(&app_id).await? {
        session.assert_current_profile().await?;
        app::upsert_board(
            session.app_handle.clone(),
            app_id.clone(),
            Some(board_id.clone()),
            board.name.clone(),
            board.description.clone(),
            None,
            None,
            None,
            Some(board.clone()),
            None,
            Some(board.updated_at),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    let context = nested_context(
        session,
        request_id,
        Some(app_id.clone()),
        Some(board_id.clone()),
    );
    let response = run_specialist(
        session,
        CopilotScope::Board,
        instruction,
        mode == "explain",
        Some(board),
        None,
        None,
        context,
        request_id,
        Vec::new(),
    )
    .await?;
    if mode == "explain" {
        return Ok(
            json!({"status":"ok","mode":"explain","app_id":app_id,"board_id":board_id,"message":response.message,"applied":false}),
        );
    }
    let Some(token) = response.flow_ir_commit else {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_BOARD_NO_COMPILED_COMMIT","app_id":app_id,"board_id":board_id,"applied":false,"specialist_message":response.message,"flowscript":response.flowscript_workspace,"message":"The specialist returned no complete validated compiler commit. No board edit was applied."}),
        );
    };
    if token.board_id != board_id {
        return Err("The compiled commit belongs to another board.".into());
    }
    session.assert_current_profile().await?;
    let job = copilot::flowpilot_create_board_edit_job(
        session.app_handle.clone(),
        app_id.clone(),
        Some(request_id.to_owned()),
        token,
    )
    .await?;
    if run_options(session.context.as_ref()).auto_approve {
        return apply_job(session, job, true).await;
    }
    Ok(
        json!({"status":"awaiting_approval","app_id":app_id,"board_id":board_id,"job_id":job.job_id,"command_count":job.review.command_count,"requires_destructive_approval":job.review.replacement_mode || !job.review.destructive_effects.is_empty(),"created_board_id":created.then_some(board_id),"applied":false,"message":"The complete workflow compiled and its exact native review is retained. Apply the review to persist the edit."}),
    )
}

pub(super) async fn review(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let app_id = specialist_app_id(session, args)?;
    session.require_visible(&app_id)?;
    let board_id = required(args, "board_id")?;
    let action = required(args, "action")?;
    if action == "list" {
        let mut jobs = copilot::flowpilot_list_board_edit_jobs(
            Some(app_id.clone()),
            Some(board_id.to_owned()),
            Some(true),
        );
        jobs.reverse();
        return Ok(
            json!({"status":"ok","app_id":app_id,"board_id":board_id,"reviews":jobs.iter().map(review_summary).collect::<Vec<_>>()}),
        );
    }
    if !["status", "apply", "dismiss"].contains(&action) {
        return Err("Unknown board review action.".into());
    }
    let job = copilot::flowpilot_get_board_edit_job(required(args, "job_id")?.to_owned())
        .ok_or("The retained board review was not found.")?;
    if job.app_id != app_id || job.board_id != board_id {
        return Err("The review does not match app_id and board_id.".into());
    }
    if action == "status" {
        let mut value = review_summary(&job);
        value["status"] = json!("ok");
        return Ok(value);
    }
    // The common native dispatcher presents the exact apply/dismiss approval before this call.
    apply_job(session, job, action == "apply").await
}

fn review_summary(job: &BoardEditJob) -> Value {
    json!({"job_id":job.job_id,"app_id":job.app_id,"board_id":job.board_id,"phase":job.phase,"applied":matches!(job.phase,BoardEditJobPhase::Applied | BoardEditJobPhase::AppliedPendingDelivery),"command_count":job.review.command_count,"command_counts":job.review.command_counts,"destructive_effects":job.review.destructive_effects,"replacement_mode":job.review.replacement_mode,"draft_id":job.token.draft_id,"revision":job.token.revision,"created_at_ms":job.created_at_ms,"updated_at_ms":job.updated_at_ms,"error":job.error,"runtime_verified":false})
}

async fn apply_job(
    session: &NativePlatformSession,
    job: BoardEditJob,
    approved: bool,
) -> Result<Value, String> {
    session.assert_current_profile().await?;
    let local = session.is_local(&job.app_id).await?;
    let resolved = copilot::flowpilot_resolve_board_edit_job(
        session.app_handle.clone(),
        job.job_id.clone(),
        approved,
        Some(true),
        (!local).then(|| session.profile.id.clone()),
        if local { None } else { session.subject.clone() },
        (!local).then(|| session.hub.clone()),
    )
    .await?;
    let job = resolved.job;
    if !matches!(
        job.phase,
        BoardEditJobPhase::AppliedPendingDelivery | BoardEditJobPhase::Applied
    ) {
        let mut result = review_summary(&job);
        result["status"] = serde_json::to_value(job.phase).map_err(|e| e.to_string())?;
        return Ok(result);
    }
    let mut result = review_summary(&job);
    result["status"] = json!("applied");
    if job.phase == BoardEditJobPhase::Applied {
        result["persisted_readback_verified"] = json!(false);
        return Ok(result);
    }
    let delivery = deliver_job(session, &job).await;
    match delivery {
        Ok(()) => {
            let board = load_board(session, &job.app_id, &job.board_id).await?;
            result["persisted_readback_verified"] = json!(true);
            result["delivery_status"] = json!("delivered");
            result["phase"] = json!("applied");
            result["event_nodes"] = inspection(&job.app_id, &board)["event_nodes"].clone();
        }
        Err(error) => {
            result["status"] = json!("error");
            result["outcome"] = json!("unknown");
            result["persisted_readback_verified"] = json!(false);
            result["delivery_status"] = json!("pending");
            result["diagnostics"] = json!([error]);
        }
    }
    Ok(result)
}

async fn deliver_job(session: &NativePlatformSession, job: &BoardEditJob) -> Result<(), String> {
    let claim = copilot::flowpilot_claim_board_edit_job_delivery(job.job_id.clone())?;
    if claim.job.phase == BoardEditJobPhase::Applied {
        return Ok(());
    }
    let lease = claim
        .delivery_lease_id
        .filter(|_| claim.claimed)
        .ok_or("Another host is delivering this review. Retry status after it settles.")?;
    let receipt = claim
        .job
        .result
        .as_ref()
        .ok_or("The retained apply receipt is unavailable.")?;
    let expected = receipt
        .persisted_board_fingerprint
        .as_deref()
        .ok_or("The retained apply receipt has no graph fingerprint.")?;
    if !session.is_local(&job.app_id).await? {
        let body = json!({"commands":receipt.commands});
        // Keep the same single atomic command body and receipt identity as the desktop adapter.
        let raw = body.to_string();
        if raw.len() > 4 * 1024 * 1024
            || serde_json::to_string(&raw)
                .map_err(|e| e.to_string())?
                .len()
                > 6 * 1024 * 1024 - 256 * 1024
        {
            return Err("The compiled batch exceeds the hub's atomic command transport. The durable review remains pending.".into());
        }
        let key = format!("flowpilot-board-edit:claim:{}:0", job.token.claim_id);
        session
            .request_with_headers(
                Method::POST,
                &format!(
                    "apps/{}/board/{}",
                    segment(&job.app_id),
                    segment(&job.board_id)
                ),
                Some(&body),
                &[("Idempotency-Key", key.as_str())],
            )
            .await?;
        let value = session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/board/{}",
                    segment(&job.app_id),
                    segment(&job.board_id)
                ),
                None,
            )
            .await?;
        let remote: Board = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if copilot::persisted_board_graph_fingerprint(&remote)? != expected {
            return Err("The hub board does not match the compiled apply receipt. The review remains pending delivery.".into());
        }
    } else {
        let readback = copilot::flowpilot_read_flow_ir_commit_board(
            session.app_handle.clone(),
            job.app_id.clone(),
            job.board_id.clone(),
        )
        .await?;
        if readback.graph_fingerprint != expected {
            return Err("The persisted board does not match the compiled apply receipt.".into());
        }
    }
    session.assert_current_profile().await?;
    copilot::flowpilot_ack_board_edit_job_delivery(
        session.app_handle.clone(),
        job.job_id.clone(),
        lease,
    )
    .await?;
    Ok(())
}

fn inspection(app_id: &str, board: &Board) -> Value {
    let source = board_to_flowscript(
        board,
        &RenderOptions {
            anchors: true,
            ..Default::default()
        },
    );
    let value = serde_json::to_value(board).expect("Board is serializable");
    let entries = crate::functions::ai::native_platform_tools::workflow_entries(&value, &board.id);
    super::board_inspection::inspect(app_id, &board.id, &value, &source, entries)
}
