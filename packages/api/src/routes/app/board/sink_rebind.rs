//! What a person writes into a board runs as that person. An edit changes what every sink that
//! runs the board at its latest version runs, so the editor becomes the user those sinks run as,
//! whether they are on or off: a sink switched back on would otherwise run the edit as whoever it
//! ran as before, with their OAuth grants. A publication changes what the sinks pinned to the new
//! version run. That is normally none, since a published version is never rewritten, but a board
//! deleted and created again publishes its old numbers once more, so the publisher becomes the
//! user those sinks run as.

use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{event, event_sink},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{app::events::db::db_model_to_event, sink::authority::bind_run_as},
    state::AppState,
};
use axum::http::StatusCode;
use flow_like::flow::event::Event;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    Select,
};

type Version = (u32, u32, u32);

/// A board write refused because the sinks that run the board could not be rebound to its
/// author before the save.
pub(crate) const SINK_REBIND_UNAVAILABLE: &str = "SINK_REBIND_UNAVAILABLE";

/// Makes the editor of a board write the run-as user of the app's sinks that run `board_id` at
/// its latest version, with the credential rules of `bind_run_as`. A write calls it before its
/// save and fails when it fails, so no sink runs what it saves as someone else, with their
/// grants; a fire that pins the saved draft then reads the editor as its run-as user. Only
/// OpenID and PAT principals edit as a person.
pub(crate) async fn try_rebind_latest_sinks(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    editor: &AppUser,
) -> Result<(), ApiError> {
    rebind(&state.db, state.db_dialect, app_id, board_id, None, editor).await
}

/// [`try_rebind_latest_sinks`] once the write is saved, for an event pointed at the board in
/// between. The write already happened, so a failure is only logged.
pub(crate) async fn rebind_latest_sinks(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    editor: &AppUser,
) {
    rebind(&state.db, state.db_dialect, app_id, board_id, None, editor)
        .await
        .ok();
}

/// Makes the publisher of `version` the run-as user of the app's sinks pinned to exactly that
/// version of `board_id`, by the rules of [`try_rebind_latest_sinks`]. The version is published
/// by then, so a failure is only logged.
pub(crate) async fn rebind_published_sinks(
    state: &AppState,
    app_id: &str,
    board_id: &str,
    version: Version,
    publisher: &AppUser,
) {
    rebind(
        &state.db,
        state.db_dialect,
        app_id,
        board_id,
        Some(version),
        publisher,
    )
    .await
    .ok();
}

#[tracing::instrument(level = "debug", skip(db, author))]
async fn rebind(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    board_id: &str,
    version: Option<Version>,
    author: &AppUser,
) -> Result<(), ApiError> {
    let Some(actor) = human_editor(author) else {
        return Ok(());
    };
    match rebind_board_sinks(db, dialect, app_id, board_id, version, actor).await {
        Ok(0) => Ok(()),
        Ok(sinks) => {
            tracing::info!(
                app_id,
                board_id,
                ?version,
                actor,
                sinks,
                "A board write made its author the run-as user of the sinks that run what it wrote"
            );
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                app_id,
                board_id,
                ?version,
                actor,
                error = %error,
                "The sinks that run this board write could not be rebound to its author"
            );
            Err(ApiError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                SINK_REBIND_UNAVAILABLE,
                format!(
                    "Your change to board {board_id} was not saved: the event triggers that run it could not be switched to run as you first. Try again shortly."
                ),
            ))
        }
    }
}

/// The bind is retried on a lost commit race, so a concurrent sink write cannot drop it.
async fn rebind_board_sinks(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    board_id: &str,
    version: Option<Version>,
    actor: &str,
) -> Result<u64, DbErr> {
    let event_ids = sink_events(db, app_id, board_id, version, actor).await?;
    if event_ids.is_empty() {
        return Ok(0);
    }
    retry_transaction(db, dialect, None, &RetryPolicy::default(), |txn| {
        let (app_id, event_ids, actor) = (app_id.to_owned(), event_ids.clone(), actor.to_owned());
        Box::pin(async move { bind_run_as(txn, &app_id, &event_ids, &actor).await })
    })
    .await
}

/// The events whose sink, on or off, runs `version` of `board_id` (`None`: latest) and needs
/// `actor` bound.
async fn sink_events<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    board_id: &str,
    version: Option<Version>,
    actor: &str,
) -> Result<Vec<String>, DbErr> {
    Ok(rebind_candidates(app_id, board_id, actor)
        .all(db)
        .await?
        .into_iter()
        .filter_map(|row| db_model_to_event(row).ok())
        .filter(|event| runs_board(event, board_id, version))
        .map(|event| event.id)
        .collect())
}

fn human_editor(user: &AppUser) -> Option<&str> {
    match user {
        AppUser::OpenID(user) => Some(&user.sub),
        AppUser::PAT(user) => Some(&user.sub),
        AppUser::APIKey(_)
        | AppUser::Executor(_)
        | AppUser::ConnectedApp(_)
        | AppUser::Unauthorized => None,
    }
}

/// The app's events that point at `board_id` or carry variants and whose sink does not already
/// run as `actor` without a legacy token, so a steady edit session writes nothing. A sink that
/// is off counts too: switched back on, it runs what was written meanwhile.
fn rebind_candidates(app_id: &str, board_id: &str, actor: &str) -> Select<event::Entity> {
    event::Entity::find()
        .inner_join(event_sink::Entity)
        .filter(event::Column::AppId.eq(app_id))
        .filter(
            Condition::any()
                .add(event::Column::BoardId.eq(board_id))
                .add(event::Column::Canary.is_not_null())
                .add(event::Column::Variants.is_not_null()),
        )
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(
            Condition::any()
                .add(event_sink::Column::RunAsUserId.is_null())
                .add(event_sink::Column::RunAsUserId.ne(actor))
                .add(event_sink::Column::PatEncrypted.is_not_null()),
        )
}

/// Whether a run of `event` can execute `version` of `board_id` (`None`: the stored flow): its
/// primary target or one of its variants, read the way dispatch reads them, names exactly that.
fn runs_board(event: &Event, board_id: &str, version: Option<Version>) -> bool {
    let runs = |target: &str, pinned: Option<Version>| target == board_id && pinned == version;
    runs(&event.board_id, event.board_version)
        || event
            .variant_set()
            .iter()
            .any(|variant| runs(&variant.board_id, variant.board_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{ApiKey, ConnectedAppUser, ExecutorUser, OpenIDUser, PATUser};
    use flow_like::flow::event::{CanaryEvent, EventVariant, EventVariantMode};
    use sea_orm::{DbBackend, QueryTrait};
    use std::{collections::HashMap, time::SystemTime};

    const BOARD: &str = "board";
    const PINNED: Option<Version> = Some((1, 0, 0));
    const OTHER_VERSION: Option<Version> = Some((1, 0, 1));
    const LIVE: EventVariantMode = EventVariantMode::Live { weight: 0.5 };

    fn event(id: &str, board_id: &str, board_version: Option<(u32, u32, u32)>) -> Event {
        Event {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "webhook".to_string(),
            notes: None,
            event_version: (0, 0, 1),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    fn variant(
        board_id: &str,
        board_version: Option<(u32, u32, u32)>,
        mode: EventVariantMode,
    ) -> EventVariant {
        EventVariant {
            name: "canary".to_string(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode,
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn canary(board_id: &str, board_version: Option<(u32, u32, u32)>) -> CanaryEvent {
        CanaryEvent {
            weight: 0.5,
            variables: HashMap::new(),
            board_id: board_id.to_string(),
            board_version,
            node_id: "node".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn with_variant(event: Event, variant: EventVariant) -> Event {
        Event {
            variants: vec![variant],
            ..event
        }
    }

    fn with_canary(event: Event, canary: CanaryEvent) -> Event {
        Event {
            canary: Some(canary),
            ..event
        }
    }

    #[test]
    fn an_edit_reaches_every_latest_target_dispatch_can_pick() {
        let shadow = EventVariantMode::Shadow { sample_rate: 0.1 };
        let cases = [
            ("primary latest", event("e", BOARD, None), true),
            ("primary pinned", event("e", BOARD, PINNED), false),
            ("other board", event("e", "other", None), false),
            (
                "variant latest",
                with_variant(event("e", "other", PINNED), variant(BOARD, None, LIVE)),
                true,
            ),
            (
                "shadow variant latest",
                with_variant(event("e", "other", None), variant(BOARD, None, shadow)),
                true,
            ),
            (
                "variant pinned",
                with_variant(event("e", "other", None), variant(BOARD, PINNED, LIVE)),
                false,
            ),
            (
                "variant latest on another board",
                with_variant(event("e", BOARD, PINNED), variant("other", None, LIVE)),
                false,
            ),
            (
                "legacy canary latest",
                with_canary(event("e", "other", PINNED), canary(BOARD, None)),
                true,
            ),
            (
                "legacy canary behind stored variants",
                with_canary(
                    with_variant(event("e", "other", None), variant(BOARD, PINNED, LIVE)),
                    canary(BOARD, None),
                ),
                false,
            ),
        ];
        for (case, event, expected) in cases {
            assert_eq!(runs_board(&event, BOARD, None), expected, "{case}");
        }
    }

    #[test]
    fn a_publication_reaches_only_the_targets_pinned_to_it() {
        let cases = [
            ("primary pinned to it", event("e", BOARD, PINNED), true),
            ("primary latest", event("e", BOARD, None), false),
            (
                "primary pinned elsewhere",
                event("e", BOARD, OTHER_VERSION),
                false,
            ),
            ("other board", event("e", "other", PINNED), false),
            (
                "variant pinned to it",
                with_variant(event("e", "other", None), variant(BOARD, PINNED, LIVE)),
                true,
            ),
            (
                "legacy canary pinned to it",
                with_canary(event("e", "other", None), canary(BOARD, PINNED)),
                true,
            ),
        ];
        for (case, event, expected) in cases {
            assert_eq!(runs_board(&event, BOARD, PINNED), expected, "{case}");
        }
    }

    #[test]
    fn a_person_edits_as_themselves() {
        let openid = AppUser::OpenID(OpenIDUser {
            sub: "person".to_string(),
            access_token: "token".to_string(),
        });
        let pat = AppUser::PAT(PATUser {
            pat: "pat".to_string(),
            sub: "person".to_string(),
        });
        assert_eq!(human_editor(&openid), Some("person"));
        assert_eq!(human_editor(&pat), Some("person"));
    }

    /// Keys, runs and app connections act for a person at most, never as the editor.
    #[test]
    fn no_other_principal_edits_as_a_person() {
        let person = || Some("person".to_string());
        let others = [
            AppUser::APIKey(ApiKey {
                key_id: "key".to_string(),
                api_key: "secret".to_string(),
                app_id: "app".to_string(),
                creator_user_id: person(),
            }),
            AppUser::Executor(ExecutorUser {
                sub: "person".to_string(),
                app_id: "app".to_string(),
                run_id: "run".to_string(),
                technical_user_id: None,
                app_chain: None,
                correlation: None,
                shadow: false,
                hosted_frontend: false,
                event_id: None,
                custom_bits_owner: person(),
                unattended: false,
            }),
            AppUser::ConnectedApp(ConnectedAppUser {
                sub: person(),
                origin_app_id: "origin".to_string(),
                target_app_id: "app".to_string(),
                app_chain: vec!["origin".to_string()],
                technical_user_id: None,
                run_id: None,
                correlation: None,
            }),
            AppUser::Unauthorized,
        ];
        for user in &others {
            assert_eq!(human_editor(user), None, "{user:?}");
        }
    }

    /// A sink that is off is a candidate too: switched back on, it runs what was written.
    #[test]
    fn candidates_are_board_and_variant_events_whose_sink_needs_the_editor() {
        let sql = rebind_candidates("app", BOARD, "editor")
            .build(DbBackend::Postgres)
            .to_string();
        for fragment in [
            r#"INNER JOIN "public"."EventSink" ON "Event"."id" = "EventSink"."eventId""#,
            r#""Event"."appId" = 'app'"#,
            r#""Event"."boardId" = 'board' OR "Event"."canary" IS NOT NULL OR "Event"."variants" IS NOT NULL"#,
            r#""EventSink"."appId" = 'app'"#,
            r#""EventSink"."runAsUserId" IS NULL OR "EventSink"."runAsUserId" <> 'editor' OR "EventSink"."patEncrypted" IS NOT NULL"#,
        ] {
            assert!(sql.contains(fragment), "{fragment} missing from {sql}");
        }
        assert!(!sql.contains(r#""EventSink"."active""#), "{sql}");
    }

    const REBIND: &str = "rebind_latest_sinks(";
    const TRY_REBIND: &str = "try_rebind_latest_sinks(";
    const LEASE_CHECK: &str = "mutation_guard.ensure_held()?;";
    const GRAPH_SAVE: &str = "save_board_and_refresh_summary(";

    /// Each human write of the board and its pages: the statement its write starts with, found
    /// as the last one before its save, and that save.
    const HUMAN_WRITES: [(&str, &str, &str, &str); 7] = [
        (
            include_str!("upsert_board.rs"),
            "pub async fn upsert_board",
            "create_missing_board(",
            "save_metadata(",
        ),
        (
            include_str!("execute_commands.rs"),
            "pub async fn execute_commands",
            LEASE_CHECK,
            GRAPH_SAVE,
        ),
        (
            include_str!("apply_flowscript.rs"),
            "pub async fn apply_flowscript",
            LEASE_CHECK,
            GRAPH_SAVE,
        ),
        (
            include_str!("flow_ir_commit.rs"),
            "pub async fn apply_flow_ir_commit",
            LEASE_CHECK,
            GRAPH_SAVE,
        ),
        (
            include_str!("undo_redo_board.rs"),
            "pub async fn undo_board",
            LEASE_CHECK,
            GRAPH_SAVE,
        ),
        (
            include_str!("undo_redo_board.rs"),
            "pub async fn redo_board",
            LEASE_CHECK,
            GRAPH_SAVE,
        ),
        (
            include_str!("../page/upsert_page.rs"),
            "pub async fn upsert_page",
            "page_id_guard.ensure_held()?;",
            "board_guard.save(None)",
        ),
    ];

    /// How each write gives up the board's lease.
    const LEASE_RELEASES: [&str; 2] = ["drop(mutation_guard)", "page_id_guard.release()"];

    fn body<'a>(source: &'a str, declaration: &str) -> &'a str {
        let body = source
            .split_once(&format!("\n{declaration}("))
            .unwrap_or_else(|| panic!("{declaration} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    fn after(body: &str, from: usize, needle: &str) -> Option<usize> {
        body[from..].find(needle).map(|found| from + found)
    }

    /// Each human write rebinds as the request's principal right before it starts writing and
    /// again after its save, both while it holds the board's lease. A fire that pins the saved
    /// draft then reads the editor as its run-as user, and the sinks run as whoever wrote last.
    /// The rebind before the write fails it when it fails, so nothing is saved that a sink would
    /// run as someone else.
    #[test]
    fn every_human_write_rebinds_before_and_after_its_save() {
        for (source, declaration, write, save) in HUMAN_WRITES {
            let body = body(source, declaration);
            let saved = body
                .find(save)
                .unwrap_or_else(|| panic!("{declaration} calls {save}"));
            let starts = body[..saved]
                .rfind(write)
                .unwrap_or_else(|| panic!("{declaration} starts its write with {write}"));
            let before = body[..starts]
                .rfind(TRY_REBIND)
                .unwrap_or_else(|| panic!("{declaration} writes before it rebinds"));
            assert!(
                body[before..].starts_with(&format!(
                    "{TRY_REBIND}&state, &app_id, &board_id, &user).await?;"
                )),
                "{declaration} writes even when its rebind fails"
            );
            assert!(
                !body[before..starts].contains("return "),
                "{declaration} rebinds only on a path that returns before it writes"
            );
            let rebound = after(body, saved, REBIND)
                .unwrap_or_else(|| panic!("{declaration} does not rebind after its save"));
            for released in LEASE_RELEASES
                .iter()
                .filter_map(|release| body.rfind(release))
            {
                assert!(
                    rebound < released,
                    "{declaration} rebinds after releasing the board's lease"
                );
            }
            for (call, _) in body.match_indices(REBIND) {
                let call = &body[call..];
                let call = &call[..call.find(')').expect("a closed call")];
                assert!(
                    call.ends_with("&user"),
                    "{declaration} rebinds as someone other than the request's principal: {call}"
                );
            }
        }
    }

    /// A replay answers a write that was saved before. Its first attempt may have been cut off
    /// between that save and its rebind, so the replay rebinds as well.
    #[test]
    fn a_replayed_write_rebinds_like_the_write_it_replays() {
        for (source, declaration, replay, answer) in [
            (
                include_str!("execute_commands.rs"),
                "pub async fn execute_commands",
                "if receipt.request_digest != *digest {",
                "ExecuteCommandsResponse::new(params.commands, sync)",
            ),
            (
                include_str!("flow_ir_commit.rs"),
                "pub async fn apply_flow_ir_commit",
                "replay_applied_receipt_from_board(&board, &scope_key, &params.token, now_ms)",
                "return Ok(Json(receipt));",
            ),
        ] {
            let body = body(source, declaration);
            let replayed = body
                .find(replay)
                .unwrap_or_else(|| panic!("{declaration} replays at {replay}"));
            let answered = after(body, replayed, answer)
                .unwrap_or_else(|| panic!("{declaration} answers a replay with {answer}"));
            assert!(
                body[replayed..answered]
                    .contains("rebind_latest_sinks(&state, &app_id, &board_id, &user)"),
                "{declaration} answers a replay without rebinding"
            );
        }
    }

    /// A publication that created a version rebinds the sinks pinned to it, as its publisher,
    /// before the comparison that may still refuse the call.
    #[test]
    fn a_publication_rebinds_the_sinks_pinned_to_the_version_it_created() {
        let publish = body(
            include_str!("version_current.rs"),
            "async fn publish_under_lease",
        );
        let created = publish.find("if created {").expect("a created branch");
        let followed = after(publish, created, "after_publication(").expect("follow-ups");
        let rebound = after(
            publish,
            followed,
            "rebind_published_sinks(state, app_id, board_id, version, user)",
        )
        .expect("the publication rebinds the sinks pinned to it");
        let compared = after(publish, rebound, "ensure_comparable(").expect("a comparison");
        assert!(rebound < compared);
    }

    /// Publishing the next version rebinds the sinks pinned to the version it published, as its
    /// publisher, once the follow-ups of every publication ran.
    #[test]
    fn publishing_the_next_version_rebinds_the_sinks_pinned_to_it() {
        let publish = body(
            include_str!("version_board.rs"),
            "pub async fn version_board",
        );
        let published = publish
            .find("create_version_returning_published(")
            .expect("a publication");
        let followed = after(publish, published, "after_publication(").expect("follow-ups");
        assert!(
            after(
                publish,
                followed,
                "rebind_published_sinks(&state, &app_id, &board_id, published, &user)"
            )
            .is_some(),
            "the publication rebinds the sinks pinned to the version it published"
        );
    }

    /// An ontology action publishes the draft it pins as the request's principal. A version it
    /// publishes, in place or as a prepared snapshot, rebinds the sinks pinned to it before the
    /// schema derivation that may still fail; restoring persisted pins publishes nothing.
    #[test]
    fn an_action_publication_rebinds_the_sinks_pinned_to_the_version_it_published() {
        let source = include_str!("../graph/actions.rs");
        let publish = body(source, "async fn ensure_action_board_published");
        assert!(publish.contains("let publish_draft = publisher.is_some();"));
        let in_place = publish
            .find("published_in_place = true;")
            .expect("an in-place publication");
        let rebound = after(
            publish,
            in_place,
            "rebind_published_sinks(state, app_id, &action.board_id, pinned, publisher)",
        )
        .expect("the publication rebinds the sinks pinned to it");
        assert!(publish[in_place..rebound].contains("(published_in_place || prepared.is_some())"));
        let derived = after(publish, rebound, "derive_action_parameter_schema(")
            .expect("the schema derivation follows");
        assert!(rebound < derived);
        let materialize = body(source, "pub(crate) async fn materialize_action_events");
        assert!(materialize.contains("Some(user),"));
        let rollback = body(source, "pub(crate) async fn rollback_action_event_changes");
        assert!(rollback.contains("&mut overwritten,\n        None,\n"));
    }

    mod postgres {
        use super::*;
        use crate::routes::app::events::db::event_to_db_model;
        use sea_orm::{
            ActiveModelTrait, Database, DatabaseConnection, DatabaseTransaction, Statement,
            TransactionTrait,
        };

        const SCHEMA: &str = r#"
            CREATE TABLE public."Event" (
                id text PRIMARY KEY, "appId" text NOT NULL, name text NOT NULL, description text,
                "eventType" text NOT NULL, active boolean NOT NULL, priority integer NOT NULL,
                "boardId" text, "boardVersion" text, "nodeId" text, "pageId" text, route text,
                "isDefault" boolean NOT NULL, "eventVersion" text NOT NULL, variables jsonb,
                config jsonb, inputs jsonb, notes jsonb, canary jsonb,
                "createdAt" timestamptz(3) NOT NULL, "updatedAt" timestamptz(3) NOT NULL,
                "executionMode" text NOT NULL, "lastSetupAt" timestamptz(3), "lastSetupError" text,
                "lastSetupVersion" text, "setupStatus" text, "correlationMappings" jsonb,
                exposure text NOT NULL, variants jsonb
            );
            CREATE TABLE public."EventSink" (
                id text PRIMARY KEY, "eventId" text NOT NULL UNIQUE, "appId" text NOT NULL,
                active boolean NOT NULL, "runAsUserId" text, "patEncrypted" text,
                "oauthTokensEncrypted" text, "profileJson" jsonb,
                "updatedAt" timestamptz(3) NOT NULL DEFAULT now()
            );
        "#;

        async fn seeded() -> (DatabaseConnection, DatabaseTransaction) {
            let url = std::env::var("FLOW_LIKE_SINK_TEST_DATABASE_URL").unwrap();
            let db = Database::connect(url).await.unwrap();
            let txn = db.begin().await.unwrap();
            txn.execute_unprepared(SCHEMA).await.unwrap();
            let events = [
                ("app", event("primary-latest", BOARD, None)),
                ("app", event("primary-pinned", BOARD, PINNED)),
                ("app", event("other-board", "other", None)),
                (
                    "app",
                    with_variant(
                        event("variant-latest", "other", PINNED),
                        variant(BOARD, None, LIVE),
                    ),
                ),
                (
                    "app",
                    with_canary(event("canary-latest", "other", PINNED), canary(BOARD, None)),
                ),
                ("app", event("paused", BOARD, None)),
                ("app", event("same-editor", BOARD, None)),
                ("app", event("steady", BOARD, None)),
                ("other-app", event("foreign", BOARD, None)),
            ];
            for (app_id, event) in &events {
                event_to_db_model(app_id, event).insert(&txn).await.unwrap();
            }
            txn.execute_unprepared(
                r#"INSERT INTO public."EventSink"
                       (id, "eventId", "appId", active, "runAsUserId", "patEncrypted", "oauthTokensEncrypted", "profileJson", "updatedAt")
                   VALUES
                       ('primary-latest', 'primary-latest', 'app', TRUE, 'previous', 'pat', 'oauth', '{"id":"profile"}', 'epoch'),
                       ('primary-pinned', 'primary-pinned', 'app', TRUE, 'previous', NULL, 'oauth', '{"id":"profile"}', 'epoch'),
                       ('other-board', 'other-board', 'app', TRUE, 'previous', NULL, 'oauth', '{"id":"profile"}', 'epoch'),
                       ('variant-latest', 'variant-latest', 'app', TRUE, NULL, 'pat', 'oauth', '{"id":"profile"}', 'epoch'),
                       ('canary-latest', 'canary-latest', 'app', TRUE, 'previous', NULL, 'oauth', '{"id":"profile"}', 'epoch'),
                       ('paused', 'paused', 'app', FALSE, 'previous', NULL, 'oauth', '{"id":"profile"}', 'epoch'),
                       ('same-editor', 'same-editor', 'app', TRUE, 'editor', 'pat', 'oauth', '{"id":"profile"}', 'epoch'),
                       ('steady', 'steady', 'app', TRUE, 'editor', NULL, 'oauth', '{"id":"profile"}', 'epoch'),
                       ('foreign', 'foreign', 'other-app', TRUE, 'previous', NULL, 'oauth', '{"id":"profile"}', 'epoch')"#,
            )
            .await
            .unwrap();
            (db, txn)
        }

        #[derive(Debug, PartialEq)]
        struct SinkRow {
            id: String,
            run_as: Option<String>,
            pat: bool,
            oauth: bool,
            profile: bool,
            touched: bool,
        }

        async fn sink_rows(txn: &DatabaseTransaction) -> Vec<SinkRow> {
            txn.query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                r#"SELECT id, "runAsUserId", "patEncrypted" IS NOT NULL AS pat,
                          "oauthTokensEncrypted" IS NOT NULL AS oauth, "profileJson" IS NOT NULL AS profile,
                          "updatedAt" > 'epoch' AS touched
                   FROM public."EventSink" ORDER BY id"#,
            ))
            .await
            .unwrap()
            .iter()
            .map(|row| {
                let flag = |name: &str| row.try_get::<bool>("", name).unwrap();
                SinkRow {
                    id: row.try_get("", "id").unwrap(),
                    run_as: row.try_get("", "runAsUserId").unwrap(),
                    pat: flag("pat"),
                    oauth: flag("oauth"),
                    profile: flag("profile"),
                    touched: flag("touched"),
                }
            })
            .collect()
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn an_edit_rebinds_the_sinks_that_run_the_board_at_latest() {
            let (_db, txn) = seeded().await;
            assert_eq!(
                sink_events(&txn, "app", BOARD, PINNED, "editor")
                    .await
                    .unwrap(),
                ["primary-pinned"],
                "a publication reaches only the sinks pinned to its version"
            );
            let mut events = sink_events(&txn, "app", BOARD, None, "editor")
                .await
                .unwrap();
            events.sort();
            assert_eq!(
                events,
                [
                    "canary-latest",
                    "paused",
                    "primary-latest",
                    "same-editor",
                    "variant-latest"
                ]
            );
            assert_eq!(
                bind_run_as(&txn, "app", &events, "editor").await.unwrap(),
                5
            );

            let row = |id: &str, run_as: &str, pat, oauth, touched| SinkRow {
                id: id.to_owned(),
                run_as: Some(run_as.to_owned()),
                pat,
                oauth,
                profile: oauth,
                touched,
            };
            let expected = vec![
                row("canary-latest", "editor", false, false, true),
                row("foreign", "previous", false, true, false),
                row("other-board", "previous", false, true, false),
                row("paused", "editor", false, false, true),
                row("primary-latest", "editor", false, false, true),
                row("primary-pinned", "previous", false, true, false),
                row("same-editor", "editor", false, true, true),
                row("steady", "editor", false, true, false),
                row("variant-latest", "editor", false, false, true),
            ];
            assert_eq!(sink_rows(&txn).await, expected);

            assert!(
                sink_events(&txn, "app", BOARD, None, "editor")
                    .await
                    .unwrap()
                    .is_empty(),
                "an unchanged binding is not written again"
            );
            txn.rollback().await.unwrap();
        }
    }

    mod sqlite {
        use super::*;
        use crate::routes::app::events::db::event_to_db_model;
        use sea_orm::{
            ActiveModelTrait, ActiveValue::Set, ConnectOptions, Database, IntoActiveModel, Schema,
            sea_query::Table,
        };

        const EDITOR: &str = "editor";
        const OWNER: &str = "owner";

        /// A SQLite stand-in for the Event table and, `with_sinks`, the EventSink table.
        async fn database(with_sinks: bool) -> DatabaseConnection {
            let mut options = ConnectOptions::new("sqlite::memory:");
            options.max_connections(1);
            let db = Database::connect(options).await.unwrap();
            db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
                .await
                .unwrap();
            let schema = Schema::new(db.get_database_backend());
            let mut sources = vec![schema.create_table_from_entity(event::Entity)];
            if with_sinks {
                sources.push(schema.create_table_from_entity(event_sink::Entity));
            }
            for source in sources {
                let mut table = Table::create();
                table.table(source.get_table_name().unwrap().clone());
                for column in source.get_columns() {
                    table.col(column.clone());
                }
                db.execute(&table).await.unwrap();
            }
            db
        }

        /// The sink of `event_id`, run as `run_as` with that person's grants and profile.
        fn sink(
            event_id: &str,
            active: bool,
            run_as: Option<&str>,
            released: bool,
        ) -> event_sink::Model {
            let now = chrono::Utc::now().fixed_offset();
            event_sink::Model {
                id: event_id.to_owned(),
                event_id: event_id.to_owned(),
                app_id: "app".to_owned(),
                sink_type: "telegram".to_owned(),
                active,
                path: None,
                auth_token: None,
                webhook_secret: None,
                cron_expression: None,
                cron_timezone: None,
                pat_encrypted: None,
                oauth_tokens_encrypted: run_as.map(|person| format!("{person}-grants")),
                profile_json: run_as.map(|person| serde_json::json!({ "id": person })),
                created_at: now,
                updated_at: now,
                method: None,
                run_as_user_id: run_as.map(str::to_owned),
                run_as_released_at: released.then_some(now),
            }
        }

        async fn seeded() -> DatabaseConnection {
            let db = database(true).await;
            for (event, sink) in [
                (
                    event("armed", BOARD, None),
                    sink("armed", true, Some(OWNER), false),
                ),
                (
                    event("paused", BOARD, None),
                    sink("paused", false, Some(OWNER), false),
                ),
                (
                    event("released", BOARD, None),
                    sink("released", false, None, true),
                ),
                (
                    event("other-board", "other", None),
                    sink("other-board", false, Some(OWNER), false),
                ),
            ] {
                event_to_db_model("app", &event).insert(&db).await.unwrap();
                event_sink::Entity::insert(sink.into_active_model().reset_all())
                    .exec(&db)
                    .await
                    .unwrap();
            }
            db
        }

        async fn stored(db: &DatabaseConnection, id: &str) -> event_sink::Model {
            event_sink::Entity::find_by_id(id)
                .one(db)
                .await
                .unwrap()
                .unwrap()
        }

        fn editor() -> AppUser {
            AppUser::OpenID(OpenIDUser {
                sub: EDITOR.to_owned(),
                access_token: "token".to_owned(),
            })
        }

        /// An edit made while a sink is off binds the editor in place of the person it ran as,
        /// drops their grants and leaves the sink off; a released sink keeps its release's mark.
        /// When that person turns the sink back on, the way the toggle does, the edit runs as
        /// them without the grants they saved before it, until they save theirs again.
        #[tokio::test]
        async fn an_edit_rebinds_the_sinks_that_are_off_and_leaves_them_off() {
            let db = seeded().await;
            rebind(&db, DbDialect::Postgres, "app", BOARD, None, &editor())
                .await
                .unwrap();

            for id in ["armed", "paused", "released"] {
                let sink = stored(&db, id).await;
                assert_eq!(sink.run_as_user_id.as_deref(), Some(EDITOR), "{id}");
                assert_eq!(sink.oauth_tokens_encrypted, None, "{id}");
                assert_eq!(sink.profile_json, None, "{id}");
            }
            assert!(stored(&db, "armed").await.active);
            let paused = stored(&db, "paused").await;
            assert!(!paused.active && paused.run_as_released_at.is_none());
            let released = stored(&db, "released").await;
            assert!(!released.active && released.run_as_released_at.is_some());
            let other = stored(&db, "other-board").await;
            assert_eq!(other.run_as_user_id.as_deref(), Some(OWNER));
            assert_eq!(
                other.oauth_tokens_encrypted.as_deref(),
                Some("owner-grants")
            );
            assert!(
                sink_events(&db, "app", BOARD, None, EDITOR)
                    .await
                    .unwrap()
                    .is_empty(),
                "an unchanged binding is not written again"
            );

            event_sink::Entity::update_many()
                .set(event_sink::ActiveModel {
                    active: Set(true),
                    run_as_released_at: Set(None),
                    ..Default::default()
                })
                .filter(event_sink::Column::Id.eq("paused"))
                .exec(&db)
                .await
                .unwrap();
            assert_eq!(
                bind_run_as(&db, "app", &["paused".to_owned()], OWNER)
                    .await
                    .unwrap(),
                1
            );
            let turned_on = stored(&db, "paused").await;
            assert_eq!(turned_on.run_as_user_id.as_deref(), Some(OWNER));
            assert_eq!(turned_on.oauth_tokens_encrypted, None);
            assert!(turned_on.active);
        }

        /// A rebind the database refuses fails the write that asked for it with a 503, before
        /// anything is saved. A principal that edits as nobody binds nobody and never fails.
        #[tokio::test]
        async fn a_rebind_that_fails_refuses_the_write() {
            let db = database(false).await;
            let refused = rebind(&db, DbDialect::Postgres, "app", BOARD, None, &editor())
                .await
                .unwrap_err();
            assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(refused.public_code(), SINK_REBIND_UNAVAILABLE);
            assert!(
                refused
                    .public_message()
                    .is_some_and(|message| message.contains("board board was not saved")),
                "{refused:?}"
            );

            let key = AppUser::APIKey(ApiKey {
                key_id: "key".to_owned(),
                api_key: "secret".to_owned(),
                app_id: "app".to_owned(),
                creator_user_id: Some(EDITOR.to_owned()),
            });
            assert!(
                rebind(&db, DbDialect::Postgres, "app", BOARD, None, &key)
                    .await
                    .is_ok()
            );
        }
    }
}
