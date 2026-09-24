use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use flow_like_types::create_id;
use sea_orm::{DatabaseTransaction, IsolationLevel};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::db::coordination::coordinate;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::explore::edition::{self, Edition};
use crate::routes::explore::hydrate;
use crate::routes::explore::model::{
    ExploreEditorState, ExploreOrderBody, PlacementInput, check_slot, ensure_kind_unchanged,
};
use crate::routes::explore::resolve::ExplorePreview;
use crate::routes::store::explore::{
    forget_pages, load_edition, parse_flag, resolve_page, viewer as preview_viewer,
};
use crate::state::AppState;

const LOCK_DOMAIN: &str = "explore-layout";
const LOCK_DRAFT: &str = "draft";
const MEDIA_UPLOAD_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePlacementBody {
    pub expected_revision: String,
    pub slot_key: String,
    /// Priority inside the slot, 0 = first choice; the end of the slot when omitted.
    pub position: Option<u32>,
    pub placement: PlacementInput,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdatePlacementBody {
    pub expected_revision: String,
    pub placement: PlacementInput,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRevisionBody {
    pub expected_revision: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeletePlacementQuery {
    /// The draft revision the editor last saw.
    pub expected_revision: String,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExplorePreviewQuery {
    /// `draft` (default) or `live`.
    pub source: Option<String>,
    /// Simulate developer mode.
    pub dev: Option<String>,
    /// Simulate a signed-in viewer.
    pub signed_in: Option<String>,
    /// `desktop` or `web` (default).
    pub platform: Option<String>,
    /// Viewer language (default `en`).
    pub language: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExploreMediaQuery {
    /// `webp` (default), `png` or `jpeg`.
    pub format: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExploreMediaUpload {
    /// Signed PUT URL, valid for one hour.
    pub url: String,
    /// Public URL of the uploaded file; `None` when the hub has no CDN, so custom artwork is unavailable.
    pub final_url: Option<String>,
}

#[derive(Clone, Debug)]
enum DraftChange {
    Create {
        id: String,
        slot_key: String,
        position: Option<u32>,
        input: PlacementInput,
    },
    Update {
        id: String,
        input: PlacementInput,
    },
    Delete {
        id: String,
    },
    Order(ExploreOrderBody),
    Publish,
    Discard,
}

fn missing_placement(id: &str) -> ApiError {
    ApiError::not_found(format!("Explore placement {id} does not exist in the draft"))
}

/// One draft mutation inside the caller's transaction: coordinate on the draft, check the revision before
/// anything is loaded, validate against the current draft, write, and close with the revision CAS. Returns
/// the LIVE revision a publish replaced, whose cached pages the caller drops after the commit.
async fn apply(
    txn: &DatabaseTransaction,
    expected: &str,
    change: &DraftChange,
    now: DateTime<Utc>,
) -> Result<Option<String>, ApiError> {
    coordinate(txn, LOCK_DOMAIN, &[LOCK_DRAFT]).await?;
    match change {
        DraftChange::Publish => {
            let replaced = edition::header(txn, Edition::Live)
                .await?
                .map_or_else(|| edition::DEFAULT_REVISION.to_owned(), |header| header.revision);
            edition::publish(txn, expected, now).await?;
            return Ok(Some(replaced));
        }
        DraftChange::Discard => {
            edition::discard(txn, expected, now).await?;
            return Ok(None);
        }
        DraftChange::Create { .. }
        | DraftChange::Update { .. }
        | DraftChange::Delete { .. }
        | DraftChange::Order(_) => {}
    }
    edition::ensure_draft(txn, expected, now).await?;
    let layout = edition::load(txn, Edition::Draft).await?.layout;
    match change {
        DraftChange::Create {
            id,
            slot_key,
            position,
            input,
        } => {
            layout.ensure_room_for_placement()?;
            layout.check_target(slot_key, &input.content)?;
            let placement = input
                .clone()
                .validated(&layout.collection_ids())?
                .into_doc(id.clone());
            let mut order: Vec<String> = layout
                .slot(slot_key)
                .map(|slot| slot.placements.iter().map(|placement| placement.id.clone()).collect())
                .unwrap_or_default();
            let index = position
                .and_then(|position| usize::try_from(position).ok())
                .map_or(order.len(), |position| position.min(order.len()));
            order.insert(index, id.clone());
            let position = i32::try_from(index).unwrap_or(i32::MAX);
            edition::insert_placement(txn, Edition::Draft, slot_key, position, &placement, now).await?;
            edition::write_slot_order(txn, Edition::Draft, slot_key, &order).await?;
        }
        DraftChange::Update { id, input } => {
            let (slot, current) = layout.find(id).ok_or_else(|| missing_placement(id))?;
            ensure_kind_unchanged(current.kind, &input.content)?;
            check_slot(&slot.key, &input.content)?;
            let mut placement = input
                .clone()
                .validated(&layout.collection_ids())?
                .into_doc(id.clone());
            placement.created_at = current.created_at;
            edition::update_placement(txn, Edition::Draft, &placement, now).await?;
        }
        DraftChange::Delete { id } => {
            let (_, placement) = layout.find(id).ok_or_else(|| missing_placement(id))?;
            let referencing: Vec<&str> = layout
                .referencing(id)
                .into_iter()
                .map(|placement| placement.name.as_str())
                .collect();
            if !referencing.is_empty() {
                return Err(ApiError::conflict(format!(
                    "{} is shown by {}; remove it there first",
                    placement.name,
                    referencing.join(", ")
                )));
            }
            edition::delete_placement(txn, Edition::Draft, id).await?;
        }
        DraftChange::Order(body) => {
            let plan = edition::validate_order(&layout, body)?;
            edition::apply_order(txn, Edition::Draft, &plan).await?;
        }
        DraftChange::Publish | DraftChange::Discard => {}
    }
    edition::cas(txn, expected, now).await?;
    Ok(None)
}

/// The editor's view: draft and live from one snapshot, statuses, placement-level changes, every referenced
/// item hydrated in one batch, and a warning per item viewers cannot see.
async fn editor_state(state: &AppState) -> Result<ExploreEditorState, ApiError> {
    let (draft, live) = state
        .transaction_with(IsolationLevel::RepeatableRead, |txn| {
            Box::pin(async move {
                Ok::<_, ApiError>((
                    edition::load(txn, Edition::Draft).await?,
                    edition::load(txn, Edition::Live).await?,
                ))
            })
        })
        .await?;
    let now = Utc::now();
    let changes = edition::diff(&draft.layout, &live.layout);
    let refs = hydrate::item_refs(state, &draft.layout).await?;
    let warnings = hydrate::ref_warnings(&draft.layout, &refs);
    let draft_revision = draft.revision().to_owned();
    let mut layout = draft.layout;
    layout.fill_statuses(now);
    let live_header = live.header;
    Ok(ExploreEditorState {
        draft_revision,
        live_revision: live_header.as_ref().map(|header| header.revision.clone()),
        published_at: live_header
            .and_then(|header| header.published_at)
            .map(|at| at.with_timezone(&Utc)),
        now,
        layout,
        refs,
        changes,
        warnings,
    })
}

async fn mutate(
    state: &AppState,
    expected: String,
    change: DraftChange,
) -> Result<Json<ExploreEditorState>, ApiError> {
    let now = Utc::now();
    let replaced = state
        .transaction(|txn| {
            let expected = expected.clone();
            let change = change.clone();
            Box::pin(async move { apply(txn, &expected, &change, now).await })
        })
        .await?;
    if let Some(revision) = replaced {
        forget_pages(state, &revision);
    }
    Ok(Json(editor_state(state).await?))
}

#[utoipa::path(
    get,
    path = "/admin/explore",
    tag = "admin",
    description = "Open the Explore layout editor: the shared draft with statuses, what changed against the live page, the items it references and warnings about items viewers cannot see.",
    responses(
        (status = 200, description = "Editor state", body = ExploreEditorState),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/explore", skip(state, user))]
pub async fn get_explore_editor(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    Ok(Json(editor_state(&state).await?))
}

#[utoipa::path(
    post,
    path = "/admin/explore/placements",
    tag = "admin",
    description = "Add a placement to a slot of the Explore draft. The slot must accept the placement's kind (and rail); rows must exist first.",
    request_body = CreatePlacementBody,
    responses(
        (status = 200, description = "Editor state with the new placement", body = ExploreEditorState),
        (status = 400, description = "The placement is invalid or the slot does not accept it"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 409, description = "The draft changed; reload it before saving")
    )
)]
#[tracing::instrument(name = "POST /admin/explore/placements", skip(state, user, body))]
pub async fn create_explore_placement(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreatePlacementBody>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    let change = DraftChange::Create {
        id: create_id(),
        slot_key: body.slot_key,
        position: body.position,
        input: body.placement,
    };
    mutate(&state, body.expected_revision, change).await
}

#[utoipa::path(
    put,
    path = "/admin/explore/placements/{id}",
    tag = "admin",
    description = "Replace a draft placement's name, schedule, audience, content and items. Its kind cannot change, and the new content must still fit its slot.",
    params(("id" = String, Path, description = "Placement id")),
    request_body = UpdatePlacementBody,
    responses(
        (status = 200, description = "Editor state", body = ExploreEditorState),
        (status = 400, description = "The placement is invalid, changes kind or no longer fits its slot"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 404, description = "No such placement in the draft"),
        (status = 409, description = "The draft changed; reload it before saving")
    )
)]
#[tracing::instrument(name = "PUT /admin/explore/placements/{id}", skip(state, user, body))]
pub async fn update_explore_placement(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdatePlacementBody>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    let change = DraftChange::Update {
        id,
        input: body.placement,
    };
    mutate(&state, body.expected_revision, change).await
}

#[utoipa::path(
    delete,
    path = "/admin/explore/placements/{id}",
    tag = "admin",
    description = "Delete a draft placement and its items. A collection that a spotlight still shows cannot be deleted.",
    params(("id" = String, Path, description = "Placement id"), DeletePlacementQuery),
    responses(
        (status = 200, description = "Editor state", body = ExploreEditorState),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 404, description = "No such placement in the draft"),
        (status = 409, description = "The draft changed, or another placement still shows this collection")
    )
)]
#[tracing::instrument(name = "DELETE /admin/explore/placements/{id}", skip(state, user, query))]
pub async fn delete_explore_placement(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<DeletePlacementQuery>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    mutate(&state, query.expected_revision, DraftChange::Delete { id }).await
}

#[utoipa::path(
    put,
    path = "/admin/explore/order",
    tag = "admin",
    description = "Reorder the draft: the complete row list plus the full priority list of every slot the move touches, including both the source and the target slot.",
    request_body = ExploreOrderBody,
    responses(
        (status = 200, description = "Editor state", body = ExploreEditorState),
        (status = 400, description = "The lists are not a permutation of the touched slots, or a slot does not accept a placement"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 409, description = "The draft changed; reload it before saving")
    )
)]
#[tracing::instrument(name = "PUT /admin/explore/order", skip(state, user, body))]
pub async fn order_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ExploreOrderBody>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    mutate(&state, body.expected_revision.clone(), DraftChange::Order(body)).await
}

#[utoipa::path(
    post,
    path = "/admin/explore/publish",
    tag = "admin",
    description = "Publish the draft as the live Explore page after validating all of it. Items that no longer exist or are private do not block publishing; they are listed in `warnings`.",
    request_body = ExploreRevisionBody,
    responses(
        (status = 200, description = "Editor state after publishing, with warnings", body = ExploreEditorState),
        (status = 400, description = "The draft breaks a layout rule"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 409, description = "The draft changed; reload it before publishing")
    )
)]
#[tracing::instrument(name = "POST /admin/explore/publish", skip(state, user, body))]
pub async fn publish_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ExploreRevisionBody>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    mutate(&state, body.expected_revision, DraftChange::Publish).await
}

#[utoipa::path(
    post,
    path = "/admin/explore/discard",
    tag = "admin",
    description = "Throw away unpublished changes: the draft becomes a copy of the live page, or of the built-in default while nothing is published.",
    request_body = ExploreRevisionBody,
    responses(
        (status = 200, description = "Editor state", body = ExploreEditorState),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required"),
        (status = 409, description = "The draft changed; reload it before discarding")
    )
)]
#[tracing::instrument(name = "POST /admin/explore/discard", skip(state, user, body))]
pub async fn discard_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ExploreRevisionBody>,
) -> Result<Json<ExploreEditorState>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    mutate(&state, body.expected_revision, DraftChange::Discard).await
}

fn preview_edition(raw: Option<&str>) -> Result<Edition, ApiError> {
    match raw.map(str::trim) {
        None | Some("" | "draft") => Ok(Edition::Draft),
        Some("live") => Ok(Edition::Live),
        Some(other) => Err(ApiError::bad_request(format!(
            "source value '{other}' is not supported; expected draft or live"
        ))),
    }
}

#[utoipa::path(
    get,
    path = "/admin/explore/preview",
    tag = "admin",
    description = "Resolve the draft or live Explore page as a simulated viewer, with a trace of which placement each slot chose and why the others were skipped.",
    params(ExplorePreviewQuery),
    responses(
        (status = 200, description = "Resolved page and selection trace", body = ExplorePreview),
        (status = 400, description = "Unknown source, platform, language or flag value"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/explore/preview", skip(state, user))]
pub async fn preview_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ExplorePreviewQuery>,
) -> Result<Json<ExplorePreview>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    let source = preview_edition(query.source.as_deref())?;
    let viewer = preview_viewer(
        &state,
        parse_flag("signed_in", query.signed_in.as_deref())?,
        query.platform.as_deref(),
        query.language.as_deref(),
        query.dev.as_deref(),
    )?;
    let loaded = load_edition(&state, source).await?;
    let (page, trace) = resolve_page(&state, &loaded, &viewer, Utc::now()).await?;
    Ok(Json(ExplorePreview { page, trace }))
}

fn media_extension(format: Option<&str>) -> Result<&'static str, ApiError> {
    match format.unwrap_or("webp") {
        "webp" => Ok("webp"),
        "png" => Ok("png"),
        "jpeg" => Ok("jpg"),
        other => Err(ApiError::bad_request(format!(
            "Explore artwork must use webp, png or jpeg format (got '{other}')"
        ))),
    }
}

#[utoipa::path(
    get,
    path = "/admin/explore/media",
    tag = "admin",
    description = "Get a one-hour upload URL for custom Explore artwork and the public URL the file will have.",
    params(ExploreMediaQuery),
    responses(
        (status = 200, description = "Signed upload URL", body = ExploreMediaUpload),
        (status = 400, description = "Unsupported image format"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — landing page permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/explore/media", skip(state, user))]
pub async fn sign_explore_media(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ExploreMediaQuery>,
) -> Result<Json<ExploreMediaUpload>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteLandingPage)
        .await?;
    let extension = media_extension(query.format.as_deref())?;
    let path = flow_like_storage::object_store::path::Path::from("explore")
        .join(format!("{}.{extension}", create_id()));
    let url = state
        .cdn_bucket
        .sign("PUT", &path, MEDIA_UPLOAD_TTL)
        .await?;
    let final_url = state
        .platform_config
        .cdn
        .as_ref()
        .filter(|url| !url.trim().is_empty())
        .map(|url| format!("{}/{}", url.trim_end_matches('/'), path));
    Ok(Json(ExploreMediaUpload {
        url: url.to_string(),
        final_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn artwork_formats_default_to_webp() {
        assert_eq!(media_extension(None).unwrap(), "webp");
        assert_eq!(media_extension(Some("jpeg")).unwrap(), "jpg");
        for invalid in ["", "svg", "../png"] {
            assert_eq!(
                media_extension(Some(invalid)).unwrap_err().status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn preview_sources_parse() {
        assert_eq!(preview_edition(None).unwrap(), Edition::Draft);
        assert_eq!(preview_edition(Some("live")).unwrap(), Edition::Live);
        assert!(preview_edition(Some("staging")).is_err());
    }

    #[test]
    fn bodies_reject_unknown_fields() {
        let body = serde_json::json!({
            "expectedRevision": "default",
            "slotKey": "row:trending",
            "placement": {
                "kind": "rail",
                "name": "Trending",
                "enabled": true,
                "startsAt": null,
                "endsAt": null,
                "audience": [],
                "content": {"kind": "rail", "rail": "trending", "title": null},
                "items": []
            }
        });
        let error = serde_json::from_value::<CreatePlacementBody>(body.clone()).unwrap_err();
        assert!(error.to_string().contains("kind"), "{error}");
        let mut body = body;
        body["placement"].as_object_mut().unwrap().remove("kind");
        let parsed = serde_json::from_value::<CreatePlacementBody>(body).unwrap();
        assert_eq!(parsed.position, None);
        assert!(
            serde_json::from_value::<ExploreRevisionBody>(serde_json::json!({
                "expectedRevision": "r",
                "force": true
            }))
            .is_err()
        );
    }

    mod database {
        use super::*;
        use crate::entity::explore_placement_item;
        use crate::routes::explore::model::{
            ExploreSlotOrder, ItemKind, ItemOverrides, LayoutDoc, PlacementContent,
            PlacementItemDoc, RailKey, SLOT_HERO, SLOT_STAT, SLOT_UNPLACED,
        };
        use crate::routes::explore::query::test_database::Fixture;
        use flow_like_types::tokio;
        use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

        async fn run(
            db: &DatabaseConnection,
            expected: &str,
            change: DraftChange,
        ) -> Result<Option<String>, ApiError> {
            let txn = db.begin().await.unwrap();
            let result = apply(&txn, expected, &change, Utc::now()).await;
            if result.is_ok() {
                txn.commit().await.unwrap();
            } else {
                txn.rollback().await.unwrap();
            }
            result
        }

        async fn draft(db: &DatabaseConnection) -> (String, LayoutDoc) {
            let loaded = edition::load(db, Edition::Draft).await.unwrap();
            (loaded.revision().to_owned(), loaded.layout)
        }

        fn status_of<T>(result: Result<T, ApiError>) -> (StatusCode, String) {
            match result {
                Ok(_) => panic!("expected an error"),
                Err(error) => (
                    error.status(),
                    error.public_message().unwrap_or_default().to_owned(),
                ),
            }
        }

        fn ids(layout: &LayoutDoc, slot: &str) -> Vec<String> {
            layout
                .slot(slot)
                .unwrap()
                .placements
                .iter()
                .map(|placement| placement.id.clone())
                .collect()
        }

        fn input(content: PlacementContent, items: &[(ItemKind, &str)]) -> PlacementInput {
            PlacementInput {
                name: "Picks".into(),
                enabled: true,
                starts_at: None,
                ends_at: None,
                audience: Vec::new(),
                content,
                items: items
                    .iter()
                    .map(|(kind, id)| {
                        PlacementItemDoc::from_row(*kind, (*id).into(), ItemOverrides::default())
                    })
                    .collect(),
            }
        }

        fn picks() -> PlacementContent {
            PlacementContent::Collection {
                title: "Picks".into(),
                blurb: None,
                source: crate::routes::explore::model::CollectionSource::Hand,
                rule: None,
            }
        }

        async fn item_rows(db: &DatabaseConnection, id: &str) -> usize {
            explore_placement_item::Entity::find()
                .filter(explore_placement_item::Column::Edition.eq(Edition::Draft.as_str()))
                .filter(explore_placement_item::Column::PlacementId.eq(id))
                .all(db)
                .await
                .unwrap()
                .len()
        }

        #[tokio::test]
        #[ignore = "requires a disposable PostgreSQL database"]
        async fn draft_mutations_check_the_revision_and_the_layout_rules() {
            let fixture = Fixture::new().await;
            let db = &fixture.db;

            let create = |id: &str, slot: &str, position: Option<u32>, input: PlacementInput| {
                DraftChange::Create {
                    id: id.into(),
                    slot_key: slot.into(),
                    position,
                    input,
                }
            };
            let picks_input = input(picks(), &[(ItemKind::App, "a1"), (ItemKind::App, "a2")]);
            let (status, _) = status_of(
                run(db, "stale", create("picks", "row:trending", None, picks_input.clone())).await,
            );
            assert_eq!(status, StatusCode::CONFLICT);
            run(db, edition::DEFAULT_REVISION, create("picks", "row:trending", Some(0), picks_input.clone()))
                .await
                .unwrap();
            let (revision, layout) = draft(db).await;
            assert_ne!(revision, edition::DEFAULT_REVISION);
            assert_eq!(ids(&layout, "row:trending"), ["picks", "default-trending"]);
            let (status, _) = status_of(
                run(db, edition::DEFAULT_REVISION, create("late", "row:trending", None, picks_input.clone())).await,
            );
            assert_eq!(status, StatusCode::CONFLICT);

            let trending = input(
                PlacementContent::Rail {
                    rail: RailKey::Trending,
                    title: None,
                },
                &[],
            );
            let (status, message) = status_of(run(db, &revision, create("t2", SLOT_STAT, None, trending.clone())).await);
            assert_eq!((status, message.as_str()), (StatusCode::BAD_REQUEST, "stat does not accept rail/trending"));
            let (status, _) = status_of(run(db, &revision, create("t2", "row:ghost", None, trending.clone())).await);
            assert_eq!(status, StatusCode::BAD_REQUEST);

            let (status, message) = status_of(
                run(db, &revision, DraftChange::Update { id: "picks".into(), input: trending.clone() }).await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(message.starts_with("Placement kind cannot change"), "{message}");
            let (status, _) = status_of(
                run(db, &revision, DraftChange::Update { id: "ghost".into(), input: picks_input.clone() }).await,
            );
            assert_eq!(status, StatusCode::NOT_FOUND);

            let three = input(picks(), &[(ItemKind::App, "a3"), (ItemKind::Package, "p1"), (ItemKind::App, "a1")]);
            run(db, &revision, DraftChange::Update { id: "picks".into(), input: three }).await.unwrap();
            let (revision, layout) = draft(db).await;
            let items: Vec<&str> = layout.find("picks").unwrap().1.items.iter().map(|item| item.id.as_str()).collect();
            assert_eq!(items, ["a3", "p1", "a1"]);
            assert_eq!(item_rows(db, "picks").await, 3);

            let spotlight = PlacementInput {
                name: "Hero slides".into(),
                ..input(
                    PlacementContent::Spotlight {
                        rotation_seconds: 8,
                        auto_fill: false,
                    },
                    &[(ItemKind::Collection, "picks")],
                )
            };
            run(db, &revision, create("slides", SLOT_HERO, Some(0), spotlight)).await.unwrap();
            let (revision, layout) = draft(db).await;
            assert_eq!(ids(&layout, SLOT_HERO), ["slides", "default-hero"]);
            let (status, message) = status_of(run(db, &revision, DraftChange::Delete { id: "picks".into() }).await);
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(message, "Picks is shown by Hero slides; remove it there first");

            let (status, _) = status_of(
                run(
                    db,
                    &revision,
                    DraftChange::Order(ExploreOrderBody {
                        expected_revision: revision.clone(),
                        rows: layout.rows().map(|row| row.key.clone()).collect(),
                        slots: vec![ExploreSlotOrder {
                            key: SLOT_UNPLACED.into(),
                            placement_ids: vec!["picks".into()],
                        }],
                    }),
                )
                .await,
            );
            assert_eq!(status, StatusCode::BAD_REQUEST);
            run(
                db,
                &revision,
                DraftChange::Order(ExploreOrderBody {
                    expected_revision: revision.clone(),
                    rows: layout.rows().map(|row| row.key.clone()).collect(),
                    slots: vec![
                        ExploreSlotOrder {
                            key: "row:trending".into(),
                            placement_ids: vec!["default-trending".into()],
                        },
                        ExploreSlotOrder {
                            key: SLOT_UNPLACED.into(),
                            placement_ids: vec!["picks".into()],
                        },
                    ],
                }),
            )
            .await
            .unwrap();
            let (revision, layout) = draft(db).await;
            assert_eq!(ids(&layout, SLOT_UNPLACED), ["picks"]);

            run(db, &revision, DraftChange::Delete { id: "slides".into() }).await.unwrap();
            let (revision, _) = draft(db).await;
            assert_eq!(item_rows(db, "slides").await, 0);

            let (status, _) = status_of(run(db, "stale", DraftChange::Publish).await);
            assert_eq!(status, StatusCode::CONFLICT);
            let replaced = run(db, &revision, DraftChange::Publish).await.unwrap();
            assert_eq!(replaced.as_deref(), Some(edition::DEFAULT_REVISION));
            let live = edition::load(db, Edition::Live).await.unwrap();
            assert!(live.header.as_ref().unwrap().published_at.is_some());
            let (revision, published) = draft(db).await;
            assert!(edition::diff(&published, &live.layout).is_empty());

            run(db, &revision, DraftChange::Delete { id: "picks".into() }).await.unwrap();
            let (revision, layout) = draft(db).await;
            assert!(layout.find("picks").is_none());
            assert_eq!(item_rows(db, "picks").await, 0);
            let (status, _) = status_of(run(db, "stale", DraftChange::Discard).await);
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(run(db, &revision, DraftChange::Discard).await.unwrap(), None);
            let (_, discarded) = draft(db).await;
            assert!(discarded.find("picks").is_some());
            assert!(edition::diff(&discarded, &live.layout).is_empty());

            fixture.drop_database().await;
        }
    }
}
