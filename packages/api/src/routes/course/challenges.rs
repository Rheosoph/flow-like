use crate::{
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot, job},
    entity::{challenge, sea_orm_active_enums::ChallengeKind},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::access::{ensure_challenge_in_lesson, ensure_lesson_in_course},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::Value;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ChallengeUpsertBody {
    pub kind: String,
    pub prompt: String,
    pub explanation: Option<String>,
    #[schema(value_type = Object)]
    pub payload: Value,
    pub points: Option<i32>,
    pub position: Option<i32>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ChallengeView {
    pub id: String,
    pub lesson_id: String,
    pub position: i32,
    pub kind: String,
    pub prompt: String,
    pub explanation: Option<String>,
    #[schema(value_type = Object)]
    pub payload: Value,
    pub points: i32,
}

pub fn parse_kind(s: &str) -> ChallengeKind {
    match s.to_uppercase().as_str() {
        "MULTIPLE_CHOICE" => ChallengeKind::MultipleChoice,
        "BOARD_RIDDLE" => ChallengeKind::BoardRiddle,
        "EXECUTE_NODE" => ChallengeKind::ExecuteNode,
        _ => ChallengeKind::SingleChoice,
    }
}

pub fn kind_to_string(k: &ChallengeKind) -> &'static str {
    match k {
        ChallengeKind::SingleChoice => "SINGLE_CHOICE",
        ChallengeKind::MultipleChoice => "MULTIPLE_CHOICE",
        ChallengeKind::BoardRiddle => "BOARD_RIDDLE",
        ChallengeKind::ExecuteNode => "EXECUTE_NODE",
    }
}

impl From<challenge::Model> for ChallengeView {
    fn from(value: challenge::Model) -> Self {
        Self::from_model(value, true)
    }
}

fn learner_payload(kind: &ChallengeKind, payload: Value) -> Value {
    let Value::Object(mut object) = payload else {
        return payload;
    };
    match kind {
        ChallengeKind::SingleChoice | ChallengeKind::MultipleChoice => {
            object.remove("correct");
        }
        ChallengeKind::ExecuteNode => {
            object.remove("expectedOutputs");
            object.remove("expected_outputs");
        }
        ChallengeKind::BoardRiddle => {}
    }
    Value::Object(object)
}

impl ChallengeView {
    pub fn from_model(value: challenge::Model, include_solution_payload: bool) -> Self {
        let payload = if include_solution_payload {
            value.payload
        } else {
            learner_payload(&value.kind, value.payload)
        };
        Self {
            id: value.id,
            lesson_id: value.lesson_id,
            position: value.position,
            kind: kind_to_string(&value.kind).to_string(),
            prompt: value.prompt,
            explanation: value.explanation,
            payload,
            points: value.points,
        }
    }
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/lessons/{lesson_id}/challenges/{challenge_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier"),
        ("challenge_id" = String, Path, description = "Challenge identifier")
    ),
    request_body = ChallengeUpsertBody,
    responses(
        (status = 200, description = "Challenge created or updated", body = ChallengeView),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/lessons/{lesson_id}/challenges/{challenge_id}",
    skip(state, user, body)
)]
pub async fn upsert_challenge(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, lesson_id, challenge_id)): Path<(String, String, String)>,
    Json(body): Json<ChallengeUpsertBody>,
) -> Result<Json<ChallengeView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let now = chrono::Utc::now().fixed_offset();
    let existing = challenge::Entity::find_by_id(&challenge_id)
        .one(&state.db)
        .await?;
    let kind = parse_kind(&body.kind);

    if existing.is_some() {
        ensure_challenge_in_lesson(&state, &course_id, &lesson_id, &challenge_id).await?;
    } else {
        ensure_lesson_in_course(&state, &course_id, &lesson_id).await?;
    }

    let saved = state
        .transaction(|txn| {
            let challenge_id = challenge_id.clone();
            let lesson_id = lesson_id.clone();
            let body = body.clone();
            let kind = kind.clone();
            let existing = existing.clone();
            Box::pin(async move {
                // Both branches: a `202` leaves the challenge row present until
                // the drain reaches `DeleteRoot`, so re-authoring its id is an
                // update as often as an insert.
                job::cancel(txn, DeletionRoot::Challenge, &challenge_id).await?;
                let saved = if let Some(c) = existing {
                    let mut active = c.into_active_model();
                    active.kind = Set(kind);
                    active.prompt = Set(body.prompt);
                    active.explanation = Set(body.explanation);
                    active.payload = Set(body.payload);
                    active.points = Set(body.points.unwrap_or(10));
                    active.position = Set(body.position.unwrap_or(0));
                    active.updated_at = Set(now);
                    active.update(txn).await?
                } else {
                    challenge::ActiveModel {
                        id: Set(challenge_id),
                        lesson_id: Set(lesson_id),
                        kind: Set(kind),
                        prompt: Set(body.prompt),
                        explanation: Set(body.explanation),
                        payload: Set(body.payload),
                        points: Set(body.points.unwrap_or(10)),
                        position: Set(body.position.unwrap_or(0)),
                        created_at: Set(now),
                        updated_at: Set(now),
                    }
                    .insert(txn)
                    .await?
                };
                Ok::<_, ApiError>(saved)
            })
        })
        .await?;

    Ok(Json(saved.into()))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/lessons/{lesson_id}/challenges/{challenge_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier"),
        ("challenge_id" = String, Path, description = "Challenge identifier")
    ),
    responses(
        (status = 200, description = "Challenge deleted"),
        (status = 202, description = "Queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/lessons/{lesson_id}/challenges/{challenge_id}",
    skip(state, user)
)]
pub async fn delete_challenge(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, lesson_id, challenge_id)): Path<(String, String, String)>,
) -> Result<Deleted<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_challenge_in_lesson(&state, &course_id, &lesson_id, &challenge_id).await?;
    let requested_by = user.sub().ok();
    deletion::delete_now(
        &state,
        DeletionRoot::Challenge,
        &challenge_id,
        requested_by.as_deref(),
        (),
    )
    .await
}
