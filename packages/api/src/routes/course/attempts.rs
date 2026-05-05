use crate::{
    credentials::CredentialsAccess,
    entity::{
        challenge, course_module, leaderboard_opt_in, lesson, sea_orm_active_enums::ChallengeKind,
        user_challenge_attempt, user_course_enrollment,
    },
    error::ApiError,
    execution::state::{EventQuery, ExecutionRunRecord, RunStatus as ExecutionRunStatus},
    middleware::jwt::AppUser,
    routes::course::access::ensure_challenge_course_readable,
    routes::execution::progress::get_state_store,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{board::Board, pin::PinType};
use flow_like_types::{Value, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    IntoActiveModel, QueryFilter, Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AttemptSubmission {
    #[schema(value_type = Object)]
    pub submission: Value,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AttemptResult {
    pub is_correct: bool,
    pub points_awarded: i32,
    pub explanation: Option<String>,
    pub attempt_id: String,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct ChallengeAttemptView {
    pub id: String,
    pub challenge_id: String,
    #[schema(value_type = Object)]
    pub submission: Value,
    pub is_correct: bool,
    pub points_awarded: i32,
    pub attempted_at: chrono::NaiveDateTime,
}

impl From<user_challenge_attempt::Model> for ChallengeAttemptView {
    fn from(value: user_challenge_attempt::Model) -> Self {
        Self {
            id: value.id,
            challenge_id: value.challenge_id,
            submission: value.submission,
            is_correct: value.is_correct,
            points_awarded: value.points_awarded,
            attempted_at: value.attempted_at,
        }
    }
}

fn validate_choice(payload: &Value, submission: &Value) -> bool {
    let correct = payload
        .get("correct")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let chosen = submission
        .get("selected")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    if correct.len() != chosen.len() {
        return false;
    }

    let mut c: Vec<String> = correct
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let mut ch: Vec<String> = chosen
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    c.sort();
    ch.sort();
    c == ch
}

fn collect_package_ids(value: Option<&Value>) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(value) = value else {
        return out;
    };
    match value {
        Value::Array(items) => {
            for item in items {
                if let Some(package_id) = item.as_str() {
                    out.insert(package_id.to_string());
                    continue;
                }
                if let Some(object) = item.as_object() {
                    for key in ["package_id", "packageId", "id"] {
                        if let Some(package_id) = object.get(key).and_then(|v| v.as_str()) {
                            out.insert(package_id.to_string());
                            break;
                        }
                    }
                }
            }
        }
        Value::Object(object) => {
            for key in ["package_id", "packageId", "id"] {
                if let Some(package_id) = object.get(key).and_then(|v| v.as_str()) {
                    out.insert(package_id.to_string());
                }
            }
        }
        Value::String(package_id) => {
            out.insert(package_id.to_string());
        }
        _ => {}
    }
    out
}

fn run_id_from_submission(submission: &Value) -> Option<&str> {
    submission
        .get("runId")
        .or_else(|| submission.get("run_id"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
}

fn payload_string<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn enrollment_value<'a>(
    enrollment: &'a user_course_enrollment::Model,
    field: &str,
    alias: &str,
    table: Option<&str>,
    key: Option<&str>,
) -> Option<&'a str> {
    let alias_value = match field {
        "linked_app_ids" => enrollment.linked_app_ids.get(alias),
        "id_maps" => enrollment.id_maps.get(alias),
        _ => None,
    }?;
    match (table, key) {
        (Some(table), Some(key)) => alias_value
            .get(table)
            .and_then(|values| values.get(key))
            .and_then(|value| value.as_str()),
        _ => alias_value.as_str(),
    }
}

async fn course_id_for_challenge(
    state: &AppState,
    challenge: &challenge::Model,
) -> Result<String, ApiError> {
    let lesson = lesson::Entity::find_by_id(&challenge.lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let module = course_module::Entity::find_by_id(&lesson.module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(module.course_id)
}

struct ChallengeTarget {
    app_id: Option<String>,
    board_id: Option<String>,
}

async fn resolve_challenge_target(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    payload: &Value,
) -> Result<Result<ChallengeTarget, String>, ApiError> {
    let app_alias = payload_string(payload, &["appAlias", "app_alias"]);
    let direct_app_id = payload_string(payload, &["appId", "app_id"]);
    let source_board_id = payload_string(payload, &["boardId", "board_id"]);

    let enrollment = if app_alias.is_some() {
        user_course_enrollment::Entity::find()
            .filter(user_course_enrollment::Column::UserId.eq(user_id))
            .filter(user_course_enrollment::Column::CourseId.eq(course_id))
            .one(&state.db)
            .await?
    } else {
        None
    };

    let expected_app_id = match (app_alias, direct_app_id, enrollment.as_ref()) {
        (Some(alias), _, Some(enrollment)) => {
            enrollment_value(enrollment, "linked_app_ids", alias, None, None).map(ToOwned::to_owned)
        }
        (Some(_), _, None) => return Ok(Err("course app alias has not been opened".into())),
        (None, Some(app_id), _) => Some(app_id.to_string()),
        (None, None, _) => None,
    };
    if app_alias.is_some() && expected_app_id.is_none() {
        return Ok(Err("course app alias has not been opened".into()));
    }

    let expected_board_id = match (app_alias, source_board_id, enrollment.as_ref()) {
        (Some(alias), Some(board_id), Some(enrollment)) => Some(
            enrollment_value(enrollment, "id_maps", alias, Some("boards"), Some(board_id))
                .unwrap_or(board_id)
                .to_string(),
        ),
        (_, Some(board_id), _) => Some(board_id.to_string()),
        _ => None,
    };

    if expected_app_id.is_none() && expected_board_id.is_none() {
        return Ok(Err(
            "challenge is missing execution target configuration".into()
        ));
    }

    Ok(Ok(ChallengeTarget {
        app_id: expected_app_id,
        board_id: expected_board_id,
    }))
}

async fn validate_execute_run_scope(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    payload: &Value,
    run: &ExecutionRunRecord,
) -> Result<Option<String>, ApiError> {
    let target = match resolve_challenge_target(state, user_id, course_id, payload).await? {
        Ok(target) => target,
        Err(error) => return Ok(Some(error)),
    };

    if let Some(expected_app_id) = target.app_id
        && run.app_id != expected_app_id
    {
        return Ok(Some(
            "execution run does not match the challenge app".into(),
        ));
    }

    if let Some(expected_board_id) = target.board_id
        && run.board_id != expected_board_id
    {
        return Ok(Some(
            "execution run does not match the challenge board".into(),
        ));
    }

    Ok(None)
}

fn decode_pin_value(raw: &Option<Vec<u8>>) -> Value {
    let Some(raw) = raw.as_ref().filter(|raw| !raw.is_empty()) else {
        return Value::Null;
    };
    let Ok(text) = String::from_utf8(raw.clone()) else {
        return Value::Null;
    };
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

fn board_to_submission(app_id: &str, board: &Board) -> Value {
    let mut pin_owner = HashMap::new();
    for node in board.nodes.values() {
        for pin in node.pins.values() {
            pin_owner.insert(pin.id.clone(), (node.id.clone(), pin.name.clone()));
        }
    }

    let nodes = board
        .nodes
        .values()
        .map(|node| {
            let pins = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Input)
                .map(|pin| {
                    (
                        pin.name.clone(),
                        json!({ "value": decode_pin_value(&pin.default_value) }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            json!({
                "id": node.id,
                "nodeTypeId": node.name,
                "type": node.name,
                "pins": pins,
            })
        })
        .collect::<Vec<_>>();

    let mut connections = Vec::new();
    for node in board.nodes.values() {
        for pin in node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output)
        {
            for target_pin_id in &pin.connected_to {
                let Some((target_node_id, target_pin_name)) = pin_owner.get(target_pin_id) else {
                    continue;
                };
                connections.push(json!({
                    "fromNodeId": node.id,
                    "fromPin": pin.name,
                    "toNodeId": target_node_id,
                    "toPin": target_pin_name,
                }));
            }
        }
    }

    json!({
        "appId": app_id,
        "boardId": board.id,
        "nodes": nodes,
        "connections": connections,
    })
}

async fn validated_board_submission(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    payload: &Value,
) -> Result<Result<Value, String>, ApiError> {
    let target = match resolve_challenge_target(state, user_id, course_id, payload).await? {
        Ok(target) => target,
        Err(error) => return Ok(Err(error)),
    };
    let Some(app_id) = target.app_id else {
        return Ok(Err("challenge is missing app target configuration".into()));
    };
    let Some(board_id) = target.board_id else {
        return Ok(Err("challenge is missing board target configuration".into()));
    };

    let board = match state
        .scoped_board(
            user_id,
            &app_id,
            &board_id,
            state,
            None,
            CredentialsAccess::ReadApp,
        )
        .await
    {
        Ok(board) => board,
        Err(error) => {
            tracing::warn!(
                user_id = %user_id,
                app_id = %app_id,
                board_id = %board_id,
                error = %error,
                "Failed to load board for course challenge validation"
            );
            return Ok(Err("challenge board could not be verified".into()));
        }
    };

    Ok(Ok(board_to_submission(&app_id, &board)))
}

async fn validate_execute_node(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    payload: &Value,
    submission: &Value,
) -> Result<(bool, Option<String>), ApiError> {
    let required = collect_package_ids(
        payload
            .get("requiredPackages")
            .or_else(|| payload.get("required_packages"))
            .or_else(|| payload.get("packages")),
    );
    if required.is_empty() {
        return Ok((
            false,
            Some("challenge is missing required package proof configuration".into()),
        ));
    }

    let Some(run_id) = run_id_from_submission(submission) else {
        return Ok((false, Some("missing execution run proof".into())));
    };
    let store = get_state_store(state).await?;
    let Some(run) = store
        .get_run(run_id)
        .await
        .map_err(|error| ApiError::internal(format!("Failed to read execution run: {}", error)))?
    else {
        return Ok((false, Some("execution run proof was not found".into())));
    };
    if run.user_id.as_deref() != Some(user_id) {
        return Ok((
            false,
            Some("execution run does not belong to this user".into()),
        ));
    }
    if run.status != ExecutionRunStatus::Completed {
        return Ok((
            false,
            Some("execution run has not completed successfully".into()),
        ));
    }
    if let Some(error) =
        validate_execute_run_scope(state, user_id, course_id, payload, &run).await?
    {
        return Ok((false, Some(error)));
    }

    let events = store
        .get_events(EventQuery {
            run_id: run_id.to_string(),
            after_sequence: None,
            only_undelivered: false,
            limit: None,
        })
        .await
        .map_err(|error| {
            ApiError::internal(format!("Failed to read execution events: {}", error))
        })?;
    let mut returned = HashSet::new();
    for event in events {
        returned.extend(collect_package_ids(Some(&event.payload)));
        returned.extend(collect_package_ids(event.payload.get("payload")));
        returned.extend(collect_package_ids(event.payload.get("packages")));
        returned.extend(collect_package_ids(event.payload.get("returnedPackages")));
        returned.extend(collect_package_ids(event.payload.get("returned_packages")));
        returned.extend(collect_package_ids(event.payload.get("streamedPackages")));
    }

    let missing = required
        .iter()
        .filter(|package_id| !returned.contains(*package_id))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok((true, None))
    } else {
        Ok((
            false,
            Some(format!(
                "missing streamed package proof: {}",
                missing.join(", ")
            )),
        ))
    }
}

fn validate_board_riddle_submission(payload: &Value, submission: &Value) -> (bool, Option<String>) {
    let predicates = match payload.get("predicates").and_then(|p| p.as_array()) {
        Some(arr) => arr,
        None => return (false, Some("missing predicates".into())),
    };
    let nodes = submission
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    let node_type_ids: Vec<String> = nodes
        .iter()
        .filter_map(|n| {
            n.get("nodeTypeId")
                .or_else(|| n.get("type"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    for p in predicates {
        let op = p.get("op").and_then(|v| v.as_str()).unwrap_or("");
        match op {
            "requires_nodes" => {
                let required = p
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for r in required {
                    if !node_type_ids.contains(&r) {
                        return (false, Some(format!("missing required node: {}", r)));
                    }
                }
            }
            "forbids_nodes" => {
                let forbidden = p
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for f in forbidden {
                    if node_type_ids.contains(&f) {
                        return (false, Some(format!("forbidden node present: {}", f)));
                    }
                }
            }
            "max_nodes" => {
                let max = p
                    .get("args")
                    .and_then(|a| a.get(0))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(i64::MAX);
                if (nodes.len() as i64) > max {
                    return (false, Some(format!("too many nodes (max {})", max)));
                }
            }
            "min_nodes" => {
                let min = p
                    .get("args")
                    .and_then(|a| a.get(0))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if (nodes.len() as i64) < min {
                    return (false, Some(format!("not enough nodes (min {})", min)));
                }
            }
            "has_connection" => {
                // args: [from_node_type, to_node_type] — checks any connection
                // exists between a node of from_node_type and one of to_node_type.
                let args = p
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                let from_t = args.first().and_then(|v| v.as_str()).unwrap_or("");
                let to_t = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let connections = submission
                    .get("connections")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default();
                let id_to_type: std::collections::HashMap<String, String> = nodes
                    .iter()
                    .filter_map(|n| {
                        let id = n.get("id").and_then(|v| v.as_str())?.to_string();
                        let t = n
                            .get("nodeTypeId")
                            .or_else(|| n.get("type"))
                            .and_then(|v| v.as_str())?
                            .to_string();
                        Some((id, t))
                    })
                    .collect();
                let ok = connections.iter().any(|c| {
                    let from_id = c.get("fromNodeId").and_then(|v| v.as_str()).unwrap_or("");
                    let to_id = c.get("toNodeId").and_then(|v| v.as_str()).unwrap_or("");
                    id_to_type.get(from_id).map(|s| s.as_str()) == Some(from_t)
                        && id_to_type.get(to_id).map(|s| s.as_str()) == Some(to_t)
                });
                if !ok {
                    return (
                        false,
                        Some(format!("missing connection: {} → {}", from_t, to_t)),
                    );
                }
            }
            "pin_value_equals" => {
                // args: [nodeTypeId, pinName, expectedValue]
                let args = p
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                let node_type = args.first().and_then(|v| v.as_str()).unwrap_or("");
                let pin_name = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let expected = args.get(2).cloned().unwrap_or(serde_json::Value::Null);
                let ok = nodes.iter().any(|n| {
                    let t = n
                        .get("nodeTypeId")
                        .or_else(|| n.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if t != node_type {
                        return false;
                    }
                    let pin_value = n
                        .get("pins")
                        .and_then(|p| p.get(pin_name))
                        .and_then(|p| p.get("value"));
                    pin_value == Some(&expected)
                });
                if !ok {
                    return (
                        false,
                        Some(format!(
                            "node {} pin {} doesn't have the expected value",
                            node_type, pin_name
                        )),
                    );
                }
            }
            _ => {}
        }
    }
    (true, None)
}

async fn validate_board_riddle(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    payload: &Value,
) -> Result<(bool, Option<String>), ApiError> {
    let submission = match validated_board_submission(state, user_id, course_id, payload).await? {
        Ok(submission) => submission,
        Err(error) => return Ok((false, Some(error))),
    };
    Ok(validate_board_riddle_submission(payload, &submission))
}

#[utoipa::path(
    post,
    path = "/courses/challenges/{challenge_id}/attempt",
    tag = "courses",
    params(("challenge_id" = String, Path, description = "Challenge identifier")),
    request_body = AttemptSubmission,
    responses(
        (status = 200, description = "Records the attempt and returns whether the submission was correct", body = AttemptResult)
    )
)]
#[tracing::instrument(
    name = "POST /courses/challenges/{challenge_id}/attempt",
    skip(state, user, body)
)]
pub async fn submit_attempt(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(challenge_id): Path<String>,
    Json(body): Json<AttemptSubmission>,
) -> Result<Json<AttemptResult>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().naive_utc();
    let challenge = ensure_challenge_course_readable(&state, &user, &challenge_id).await?;
    let course_id = course_id_for_challenge(&state, &challenge).await?;
    let txn = state.db.begin().await?;

    // Score calculation must be serialized per learner/challenge; otherwise
    // concurrent correct submissions could both observe zero prior points.
    txn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))",
        [sub.clone().into(), challenge_id.clone().into()],
    ))
    .await?;

    let (is_correct, explanation_override) = match challenge.kind {
        ChallengeKind::SingleChoice | ChallengeKind::MultipleChoice => {
            (validate_choice(&challenge.payload, &body.submission), None)
        }
        ChallengeKind::ExecuteNode => {
            validate_execute_node(
                &state,
                &sub,
                &course_id,
                &challenge.payload,
                &body.submission,
            )
            .await?
        }
        ChallengeKind::BoardRiddle => {
            validate_board_riddle(&state, &sub, &course_id, &challenge.payload).await?
        }
    };

    let max_points = challenge.points.max(0);
    let current_score = if is_correct { max_points } else { 0 };
    let previously_awarded = user_challenge_attempt::Entity::find()
        .filter(user_challenge_attempt::Column::UserId.eq(&sub))
        .filter(user_challenge_attempt::Column::ChallengeId.eq(&challenge_id))
        .all(&txn)
        .await?
        .into_iter()
        .map(|attempt| attempt.points_awarded.max(0))
        .sum::<i32>()
        .min(max_points);
    let points_awarded = (current_score - previously_awarded).max(0);

    let attempt_id = create_id();
    let active = user_challenge_attempt::ActiveModel {
        id: Set(attempt_id.clone()),
        user_id: Set(sub.clone()),
        challenge_id: Set(challenge_id),
        submission: Set(body.submission),
        is_correct: Set(is_correct),
        points_awarded: Set(points_awarded),
        attempted_at: Set(now),
    };
    active.insert(&txn).await?;

    if is_correct && points_awarded > 0 {
        let opt_in = leaderboard_opt_in::Entity::find_by_id(&sub)
            .one(&txn)
            .await?;
        if let Some(o) = opt_in {
            if o.is_opted_in {
                let mut active = o.into_active_model();
                let current = match active.total_points {
                    sea_orm::ActiveValue::Set(v) => v,
                    sea_orm::ActiveValue::Unchanged(v) => v,
                    _ => 0,
                };
                active.total_points = Set(current + points_awarded);
                active.updated_at = Set(now);
                active.update(&txn).await?;
            }
        }
    }

    txn.commit().await?;

    Ok(Json(AttemptResult {
        is_correct,
        points_awarded,
        explanation: explanation_override.or(challenge.explanation),
        attempt_id,
    }))
}
