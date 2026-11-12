use crate::{
    db::{DEFAULT_WRITE_CHUNK, DbDialect, delete_in_batches},
    entity::{event, page},
    error::ApiError,
};
use axum::http::StatusCode;
use flow_like_storage::{
    Path,
    object_store::{Error as StoreError, ObjectStore},
};
use futures::{StreamExt, TryStreamExt};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};

pub(crate) const BOARD_CLEANUP_FAILED: &str = "BOARD_CLEANUP_FAILED";

pub(crate) trait BoardCleanupSteps {
    async fn remove_event(&mut self, event_id: &str) -> Result<(), ApiError>;
    async fn remove_pages(&mut self) -> Result<(), ApiError>;
}

fn step_failed(board_id: &str, step: &str, cause: &ApiError) -> ApiError {
    let detail = cause
        .public_message()
        .map(str::to_owned)
        .unwrap_or_else(|| cause.to_string());
    tracing::error!("Deleting board {board_id}: {step} failed: {detail}");
    ApiError::coded(
        StatusCode::INTERNAL_SERVER_ERROR,
        BOARD_CLEANUP_FAILED,
        format!(
            "Board {board_id} was not deleted: {step} failed ({detail}). Retry the delete to finish the clean-up."
        ),
    )
}

/// Runs every dependant clean-up of a board, stopping at the first failure so the caller can leave
/// the board in place. Each step is idempotent, so a retry resumes where the last attempt stopped.
pub(crate) async fn clean_up_board(
    steps: &mut impl BoardCleanupSteps,
    board_id: &str,
    event_ids: &[String],
) -> Result<(), ApiError> {
    for event_id in event_ids {
        steps
            .remove_event(event_id)
            .await
            .map_err(|err| step_failed(board_id, &format!("removing event {event_id}"), &err))?;
    }
    steps
        .remove_pages()
        .await
        .map_err(|err| step_failed(board_id, "removing its pages", &err))
}

/// Events that die with a board: the ones it runs, and the ones that serve a page it owns. A page
/// event's own `board_id` is only metadata, so an event naming this board but serving another
/// board's page stays.
pub(crate) async fn events_of_board(
    db: &DatabaseConnection,
    app_id: &str,
    board_id: &str,
) -> Result<Vec<String>, ApiError> {
    let owned_pages: Vec<String> = page::Entity::find()
        .select_only()
        .column(page::Column::Id)
        .filter(page::Column::AppId.eq(app_id))
        .filter(page::Column::BoardId.eq(board_id))
        .into_tuple()
        .all(db)
        .await?;
    let mut belongs = Condition::any().add(
        Condition::all()
            .add(event::Column::BoardId.eq(board_id))
            .add(event::Column::PageId.is_null()),
    );
    if !owned_pages.is_empty() {
        belongs = belongs.add(event::Column::PageId.is_in(owned_pages));
    }
    let ids: Vec<String> = event::Entity::find()
        .select_only()
        .column(event::Column::Id)
        .filter(event::Column::AppId.eq(app_id))
        .filter(belongs)
        .into_tuple()
        .all(db)
        .await?;
    Ok(ids)
}

/// Page payloads live in `apps/{app_id}/_{board_id}/`. A missing folder is a board without pages.
pub(crate) async fn delete_page_folder(
    store: &dyn ObjectStore,
    app_id: &str,
    board_id: &str,
) -> Result<(), ApiError> {
    let folder = Path::from("apps").join(app_id).join(format!("_{board_id}"));
    let locations: Vec<Path> = match store
        .list(Some(&folder))
        .map_ok(|meta| meta.location)
        .try_collect()
        .await
    {
        Ok(locations) => locations,
        Err(StoreError::NotFound { .. }) => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let results: Vec<_> = store
        .delete_stream(futures::stream::iter(locations.into_iter().map(Ok)).boxed())
        .collect()
        .await;
    for result in results {
        match result {
            Ok(_) | Err(StoreError::NotFound { .. }) => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

pub(crate) async fn delete_page_rows(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app_id: &str,
    board_id: &str,
) -> Result<(), ApiError> {
    delete_in_batches::<page::Entity>(
        db,
        dialect,
        Condition::all()
            .add(page::Column::AppId.eq(app_id))
            .add(page::Column::BoardId.eq(board_id)),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::app::events::db::event_to_db_model;
    use flow_like::flow::event::Event;
    use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, Schema,
        sea_query::Table,
    };
    use std::{collections::HashMap, time::SystemTime};

    const APP: &str = "app";
    const BOARD: &str = "board";

    fn core_event(id: &str, board_id: &str, page_id: Option<&str>) -> Event {
        Event {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            board_id: board_id.to_string(),
            board_version: None,
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
            default_page_id: page_id.map(str::to_string),
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(event::Entity),
            schema.create_table_from_entity(page::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    async fn insert_event(db: &DatabaseConnection, event: &Event) {
        event_to_db_model(APP, event).insert(db).await.unwrap();
    }

    async fn insert_page(db: &DatabaseConnection, id: &str, board_id: &str) {
        let now = chrono::Utc::now().fixed_offset();
        page::ActiveModel {
            id: Set(id.to_string()),
            name: Set(id.to_string()),
            app_id: Set(APP.to_string()),
            board_id: Set(Some(board_id.to_string())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn sorted(db: &DatabaseConnection, board_id: &str) -> Vec<String> {
        let mut ids = events_of_board(db, APP, board_id).await.unwrap();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn only_events_that_die_with_the_board_are_selected() {
        let db = database().await;
        insert_page(&db, "own-page", BOARD).await;
        insert_page(&db, "foreign-page", "other").await;
        for event in [
            core_event("runs-board", BOARD, None),
            core_event("serves-own-page", "other", Some("own-page")),
            core_event("serves-foreign-page", BOARD, Some("foreign-page")),
            core_event("other-board", "other", None),
        ] {
            insert_event(&db, &event).await;
        }

        assert_eq!(
            sorted(&db, BOARD).await,
            vec!["runs-board".to_string(), "serves-own-page".to_string()]
        );
        assert_eq!(
            sorted(&db, "other").await,
            vec!["other-board".to_string(), "serves-foreign-page".to_string()]
        );
    }

    #[tokio::test]
    async fn events_of_another_app_are_never_selected() {
        let db = database().await;
        let mut foreign = event_to_db_model("elsewhere", &core_event("foreign", BOARD, None));
        foreign.app_id = Set("elsewhere".to_string());
        foreign.insert(&db).await.unwrap();

        assert!(sorted(&db, BOARD).await.is_empty());
    }

    #[tokio::test]
    async fn page_rows_of_the_board_go_and_other_boards_keep_theirs() {
        let db = database().await;
        insert_page(&db, "mine", BOARD).await;
        insert_page(&db, "theirs", "other").await;

        delete_page_rows(&db, DbDialect::Postgres, APP, BOARD)
            .await
            .unwrap();
        delete_page_rows(&db, DbDialect::Postgres, APP, BOARD)
            .await
            .unwrap();

        let left: Vec<String> = page::Entity::find()
            .select_only()
            .column(page::Column::Id)
            .into_tuple()
            .all(&db)
            .await
            .unwrap();
        assert_eq!(left, vec!["theirs".to_string()]);
    }

    #[tokio::test]
    async fn the_page_folder_of_the_board_is_emptied_and_siblings_stay() {
        let store = InMemory::new();
        let mine = Path::from("apps/app/_board/p1.page");
        let sibling = Path::from("apps/app/_board2/p1.page");
        let board_file = Path::from("apps/app/board.board");
        for path in [&mine, &sibling, &board_file] {
            store.put(path, "x".into()).await.unwrap();
        }

        delete_page_folder(&store, APP, BOARD).await.unwrap();
        delete_page_folder(&store, APP, BOARD).await.unwrap();

        assert!(store.head(&mine).await.is_err());
        assert!(store.head(&sibling).await.is_ok());
        assert!(store.head(&board_file).await.is_ok());
    }

    #[derive(Default)]
    struct Fake {
        removed: Vec<String>,
        pages_removed: u32,
        failing_event: Option<String>,
        failing_pages: bool,
    }

    impl BoardCleanupSteps for Fake {
        async fn remove_event(&mut self, event_id: &str) -> Result<(), ApiError> {
            if self.failing_event.as_deref() == Some(event_id) {
                return Err(ApiError::internal("schedule service unreachable"));
            }
            self.removed.push(event_id.to_string());
            Ok(())
        }

        async fn remove_pages(&mut self) -> Result<(), ApiError> {
            if self.failing_pages {
                return Err(ApiError::coded(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "X",
                    "store offline",
                ));
            }
            self.pages_removed += 1;
            Ok(())
        }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[tokio::test]
    async fn every_event_and_the_pages_are_cleaned_in_order() {
        let mut fake = Fake::default();
        clean_up_board(&mut fake, BOARD, &ids(&["a", "b"]))
            .await
            .unwrap();
        assert_eq!(fake.removed, ids(&["a", "b"]));
        assert_eq!(fake.pages_removed, 1);
    }

    #[tokio::test]
    async fn a_failing_event_stops_the_clean_up_and_names_step_and_id() {
        let mut fake = Fake {
            failing_event: Some("b".to_string()),
            ..Default::default()
        };
        let err = clean_up_board(&mut fake, BOARD, &ids(&["a", "b", "c"]))
            .await
            .unwrap_err();

        assert_eq!(err.public_code(), BOARD_CLEANUP_FAILED);
        let message = err.public_message().unwrap();
        assert!(message.contains("removing event b"), "{message}");
        assert!(message.contains(BOARD), "{message}");
        assert_eq!(fake.removed, ids(&["a"]));
        assert_eq!(fake.pages_removed, 0);
    }

    #[tokio::test]
    async fn a_failing_page_step_is_named() {
        let mut fake = Fake {
            failing_pages: true,
            ..Default::default()
        };
        let err = clean_up_board(&mut fake, BOARD, &ids(&["a"]))
            .await
            .unwrap_err();
        let message = err.public_message().unwrap();
        assert!(message.contains("removing its pages"), "{message}");
        assert!(message.contains("store offline"), "{message}");
    }

    #[tokio::test]
    async fn a_repeat_after_a_partial_failure_completes() {
        let mut fake = Fake {
            failing_event: Some("b".to_string()),
            ..Default::default()
        };
        let events = ids(&["a", "b"]);
        assert!(clean_up_board(&mut fake, BOARD, &events).await.is_err());

        fake.failing_event = None;
        clean_up_board(&mut fake, BOARD, &events).await.unwrap();
        assert!(fake.removed.contains(&"b".to_string()));
        assert_eq!(fake.pages_removed, 1);
    }

    #[test]
    fn the_route_cleans_up_before_it_deletes_the_board() {
        let source = include_str!("delete_board.rs");
        let cleanup = source.find("clean_up_board(&mut steps").unwrap();
        let board = source.find("app.delete_board(board_id)").unwrap();
        assert!(cleanup < board);
        assert!(source.contains("remove_event(self.state, self.app, event_id)"));
    }
}
