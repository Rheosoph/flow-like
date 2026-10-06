//! Events saved before [`EVENT_INPUTS_FORMAT`] carry inputs built by an older rule: ref keys where
//! a form needs the description and schema (a FlowPath struct then reads as key/value pairs), no
//! pin options, and possibly a sensitive default. Reads that hand events to a client rebuild such
//! inputs from the event's board and store them on its row once, so later reads are plain again.
//!
//! [`EVENT_INPUTS_FORMAT`]: flow_like::flow::event::EVENT_INPUTS_FORMAT

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
};

use flow_like::flow::{board::Board, event::Event};
use futures::stream::{self, StreamExt};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
    prelude::DateTimeWithTimeZone,
};

use super::db::db_model_to_event;
use crate::{entity::event, state::AppState};

/// Every miss decodes a whole board, so few load at once.
const BOARD_LOAD_CONCURRENCY: usize = 4;

type BoardTarget = (String, Option<(u32, u32, u32)>);

/// `events` in their order with outdated inputs rebuilt. Call it before any redaction, which may
/// clear the board target the rebuild reads. Inputs that cannot be rebuilt stay as stored.
pub async fn with_current_inputs(
    state: &AppState,
    app_id: &str,
    mut events: Vec<Event>,
) -> Vec<Event> {
    refresh_from_master_boards(state, app_id, &mut events).await;
    events
}

/// [`with_current_inputs`] for one event.
pub async fn event_with_current_inputs(state: &AppState, app_id: &str, mut event: Event) -> Event {
    refresh_from_master_boards(state, app_id, std::slice::from_mut(&mut event)).await;
    event
}

async fn refresh_from_master_boards(state: &AppState, app_id: &str, events: &mut [Event]) {
    refresh_outdated_inputs(
        &state.db,
        app_id,
        events,
        |(board_id, version)| async move {
            state
                .master_board_shared(app_id, &board_id, state, version)
                .await
                .map(|cached| cached.board.clone())
        },
    )
    .await;
}

async fn refresh_outdated_inputs<C, L, F>(db: &C, app_id: &str, events: &mut [Event], load_board: L)
where
    C: ConnectionTrait,
    L: Fn(BoardTarget) -> F,
    F: Future<Output = flow_like_types::Result<Arc<Board>>>,
{
    let targets: HashSet<BoardTarget> = events
        .iter()
        .filter(|event| rebuildable(event))
        .map(board_target)
        .collect();
    if targets.is_empty() {
        return;
    }
    let boards = load_boards(app_id, targets, &load_board).await;

    let mut rebuilt = Vec::new();
    for (index, event) in events.iter_mut().enumerate() {
        if !rebuildable(event) {
            continue;
        }
        let Some(board) = boards.get(&board_target(event)).and_then(Option::as_deref) else {
            continue;
        };
        match event.inputs_from_board(board) {
            Some(inputs) => {
                event.inputs = inputs;
                rebuilt.push(index);
            }
            None => tracing::debug!(
                app_id,
                event_id = %event.id,
                node_id = %event.node_id,
                "Event node is not on its board; serving the stored inputs"
            ),
        }
    }

    let rebuilt: Vec<&Event> = rebuilt.into_iter().map(|index| &events[index]).collect();
    store_rebuilt_inputs(db, app_id, &rebuilt).await;
}

fn rebuildable(event: &Event) -> bool {
    event.inputs_outdated() && !event.board_id.is_empty()
}

fn board_target(event: &Event) -> BoardTarget {
    (event.board_id.clone(), event.board_version)
}

/// Loads each distinct board once; a board that does not load maps to `None`.
async fn load_boards<L, F>(
    app_id: &str,
    targets: HashSet<BoardTarget>,
    load_board: &L,
) -> HashMap<BoardTarget, Option<Arc<Board>>>
where
    L: Fn(BoardTarget) -> F,
    F: Future<Output = flow_like_types::Result<Arc<Board>>>,
{
    stream::iter(targets)
        .map(|target| async move {
            let board = match load_board(target.clone()).await {
                Ok(board) => Some(board),
                Err(error) => {
                    tracing::warn!(
                        app_id,
                        board_id = %target.0,
                        board_version = ?target.1,
                        %error,
                        "Outdated event inputs cannot be rebuilt; serving the stored ones"
                    );
                    None
                }
            };
            (target, board)
        })
        .buffer_unordered(BOARD_LOAD_CONCURRENCY)
        .collect()
        .await
}

/// Writes rebuilt inputs to rows that still hold the event version and board target they were
/// rebuilt from, with inputs still outdated. An archived version therefore never writes.
async fn store_rebuilt_inputs<C: ConnectionTrait>(db: &C, app_id: &str, rebuilt: &[&Event]) {
    if rebuilt.is_empty() {
        return;
    }
    let by_id: HashMap<&str, &Event> = rebuilt
        .iter()
        .map(|event| (event.id.as_str(), *event))
        .collect();
    let rows = match event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Id.is_in(by_id.keys().copied()))
        .all(db)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Reading event rows to store rebuilt inputs failed; they are rebuilt again on the next read"
            );
            return;
        }
    };

    for row in rows {
        let Some(event) = by_id.get(row.id.as_str()).copied() else {
            continue;
        };
        let read_at = row.updated_at;
        let still_outdated = db_model_to_event(row)
            .is_ok_and(|stored| stored.inputs_outdated() && same_target(&stored, event));
        if still_outdated {
            let written = write_inputs_if_unchanged(db, app_id, event, read_at).await;
            log_write(app_id, &event.id, written);
        }
    }
}

fn log_write(app_id: &str, event_id: &str, written: flow_like_types::Result<bool>) {
    match written {
        Ok(true) => tracing::debug!(app_id, event_id, "Stored rebuilt event inputs"),
        Ok(false) => tracing::debug!(
            app_id,
            event_id,
            "Event row changed after it was read; its inputs are left to that write"
        ),
        Err(error) => tracing::warn!(
            app_id,
            event_id,
            %error,
            "Storing rebuilt event inputs failed; they are rebuilt again on the next read"
        ),
    }
}

fn same_target(stored: &Event, rebuilt: &Event) -> bool {
    stored.event_version == rebuilt.event_version
        && stored.board_id == rebuilt.board_id
        && stored.board_version == rebuilt.board_version
        && stored.node_id == rebuilt.node_id
        && stored.default_page_id == rebuilt.default_page_id
}

/// Sets the row's inputs only while its `updatedAt` is still `read_at`: every save sets it, so a
/// save after the read wins. `updatedAt` stays, since rebuilding is no edit, and no version is cut.
async fn write_inputs_if_unchanged<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
    event: &Event,
    read_at: DateTimeWithTimeZone,
) -> flow_like_types::Result<bool> {
    let inputs = if event.inputs.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&event.inputs)?)
    };
    let written = event::Entity::update_many()
        .set(event::ActiveModel {
            inputs: Set(inputs),
            ..Default::default()
        })
        .filter(event::Column::Id.eq(event.id.as_str()))
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::UpdatedAt.eq(read_at))
        .exec(db)
        .await?;
    Ok(written.rows_affected == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::app::events::db::{event_to_db_model, get_event_from_db_opt};
    use flow_like::flow::{
        event::{EVENT_INPUTS_FORMAT, EventInput},
        node::Node,
        pin::PinOptions,
        variable::VariableType,
    };
    use flow_like::flow_like_storage::Path;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const APP: &str = "app";
    const BOARD: &str = "board";
    const NODE: &str = "form-node";
    const FLOW_PATH_SCHEMA: &str = r#"{"title":"FlowPath","type":"object","properties":{"path":{"type":"string"},"store_ref":{"type":"string"}},"required":["path","store_ref"]}"#;
    const SCHEMA_KEY: &str = "7311090465327390071";
    const HELP_KEY: &str = "16248035215404677707";
    const HELP: &str = "The invoices to read";

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."Event" (
                id TEXT PRIMARY KEY, "appId" TEXT NOT NULL, name TEXT NOT NULL, description TEXT,
                "eventType" TEXT NOT NULL, active BOOLEAN NOT NULL, priority INTEGER NOT NULL,
                "boardId" TEXT, "boardVersion" TEXT, "nodeId" TEXT, "pageId" TEXT, route TEXT,
                "isDefault" BOOLEAN NOT NULL, "eventVersion" TEXT NOT NULL, variables TEXT,
                config TEXT, inputs TEXT, notes TEXT, canary TEXT, "createdAt" TEXT NOT NULL,
                "updatedAt" TEXT NOT NULL, "executionMode" TEXT NOT NULL, "lastSetupAt" TEXT,
                "lastSetupError" TEXT, "lastSetupVersion" TEXT, "setupStatus" TEXT,
                "correlationMappings" TEXT, exposure TEXT NOT NULL, variants TEXT
            )"#,
        )
        .await
        .unwrap();
        db
    }

    /// A form node as cleanup leaves it: help and schema are keys into the board's refs.
    fn form_board() -> (Arc<Board>, [String; 2]) {
        let mut board = Board::new_detached(Some(BOARD.into()), Path::from("apps").join(APP));
        let mut node = Node::new("events_generic", "Generic Event", "", "Events");
        node.id = NODE.into();
        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);
        let files = node.add_output_pin("files", "Files", HELP_KEY, VariableType::Struct);
        files.schema = Some(SCHEMA_KEY.into());
        let files = files.id.clone();
        let secret = node
            .add_output_pin("secret", "Secret", "", VariableType::String)
            .set_default_value(Some(json!("hush")))
            .set_options(PinOptions::new().set_sensitive(true).build())
            .id
            .clone();
        board
            .refs
            .insert(SCHEMA_KEY.into(), FLOW_PATH_SCHEMA.into());
        board.refs.insert(HELP_KEY.into(), HELP.into());
        board.nodes.insert(node.id.clone(), node);
        (Arc::new(board), [files, secret])
    }

    fn stale_input(id: &str, name: &str, data_type: &str, index: u16) -> EventInput {
        EventInput {
            id: id.into(),
            name: name.into(),
            friendly_name: name.into(),
            description: String::new(),
            data_type: data_type.into(),
            value_type: "Normal".into(),
            schema: None,
            default_value: None,
            optional: false,
            index,
            sensitive: false,
            valid_values: None,
            range: None,
            step: None,
            default_omitted: false,
            inputs_format: 0,
        }
    }

    /// A form event as the old `from_pin` stored it: ref keys and the sensitive default.
    fn stale_event(id: &str, [files, secret]: &[String; 2]) -> Event {
        let mut files = stale_input(files, "files", "Struct", 1);
        files.description = HELP_KEY.into();
        files.schema = Some(SCHEMA_KEY.into());
        let mut secret = stale_input(secret, "secret", "String", 2);
        secret.default_value = Some(br#""hush""#.to_vec());
        Event {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            board_id: BOARD.into(),
            board_version: None,
            node_id: NODE.into(),
            variables: HashMap::new(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "generic_form".into(),
            notes: None,
            event_version: (0, 0, 3),
            created_at: std::time::UNIX_EPOCH,
            updated_at: std::time::UNIX_EPOCH,
            default_page_id: None,
            inputs: vec![files, secret],
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    async fn insert(db: &DatabaseConnection, event: &Event) {
        event::Entity::insert(event_to_db_model(APP, event))
            .exec(db)
            .await
            .unwrap();
    }

    async fn row(db: &DatabaseConnection, id: &str) -> event::Model {
        event::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    async fn read(db: &DatabaseConnection, id: &str) -> Event {
        get_event_from_db_opt(db, id, APP).await.unwrap().unwrap()
    }

    /// Serves `board` for its own target and counts the loads.
    fn loader(
        board: Arc<Board>,
        loads: &AtomicUsize,
    ) -> impl Fn(BoardTarget) -> std::future::Ready<flow_like_types::Result<Arc<Board>>> + '_ {
        move |(board_id, _)| {
            loads.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if board_id == board.id {
                Ok(board.clone())
            } else {
                Err(flow_like_types::anyhow!("no board {board_id}"))
            })
        }
    }

    fn assert_current(inputs: &[EventInput]) {
        let [files, secret] = inputs else {
            panic!("two inputs expected, got {inputs:?}");
        };
        assert_eq!(files.schema.as_deref(), Some(FLOW_PATH_SCHEMA));
        assert_eq!(files.description, HELP);
        assert!(secret.sensitive && secret.default_omitted);
        assert_eq!(
            secret.default_value, None,
            "a sensitive default is withheld"
        );
        assert!(
            inputs
                .iter()
                .all(|input| input.inputs_format == EVENT_INPUTS_FORMAT)
        );
    }

    #[tokio::test]
    async fn stale_inputs_are_rebuilt_and_stored_once_without_a_new_version() {
        let db = database().await;
        let (board, pins) = form_board();
        insert(&db, &stale_event("form", &pins)).await;
        let before = row(&db, "form").await;
        let loads = AtomicUsize::new(0);

        let mut events = vec![read(&db, "form").await];
        assert!(events[0].inputs_outdated());
        refresh_outdated_inputs(&db, APP, &mut events, loader(board.clone(), &loads)).await;
        assert_current(&events[0].inputs);
        assert_eq!(loads.load(Ordering::SeqCst), 1);

        let after = row(&db, "form").await;
        assert_eq!(
            (&after.event_version, after.updated_at),
            (&before.event_version, before.updated_at),
            "a rebuild is no edit"
        );
        let stored = read(&db, "form").await;
        assert_eq!(stored.inputs, events[0].inputs);

        let mut again = vec![stored];
        refresh_outdated_inputs(&db, APP, &mut again, loader(board, &loads)).await;
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "a current event loads no board"
        );
    }

    #[tokio::test]
    async fn events_on_one_board_version_share_a_single_load() {
        let db = database().await;
        let (board, pins) = form_board();
        let mut pinned = stale_event("pinned", &pins);
        pinned.board_version = Some((1, 0, 0));
        for event in [stale_event("a", &pins), stale_event("b", &pins), pinned] {
            insert(&db, &event).await;
        }
        let mut events = vec![
            read(&db, "a").await,
            read(&db, "b").await,
            read(&db, "pinned").await,
        ];
        let loads = AtomicUsize::new(0);
        refresh_outdated_inputs(&db, APP, &mut events, loader(board, &loads)).await;
        assert_eq!(
            loads.load(Ordering::SeqCst),
            2,
            "one load per board and version"
        );
        let ids: Vec<&str> = events.iter().map(|event| event.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "pinned"], "order is kept");
        for event in &events {
            assert_current(&event.inputs);
            assert_current(&read(&db, &event.id).await.inputs);
        }
    }

    #[tokio::test]
    async fn a_row_saved_after_the_read_keeps_what_that_save_wrote() {
        let (board, pins) = form_board();
        let stale = stale_event("form", &pins);
        let mut retargeted = stale.clone();
        retargeted.board_version = Some((2, 0, 0));
        let mut resaved = stale.clone();
        resaved.inputs = stale.inputs_from_board(&board).unwrap();
        resaved.inputs[0].description = "Saved meanwhile".into();

        for saved in [retargeted, resaved] {
            let db = database().await;
            insert(&db, &stale).await;
            let mut events = vec![read(&db, "form").await];
            event::Entity::update(event_to_db_model(APP, &saved))
                .exec(&db)
                .await
                .unwrap();

            let loads = AtomicUsize::new(0);
            refresh_outdated_inputs(&db, APP, &mut events, loader(board.clone(), &loads)).await;
            assert_current(&events[0].inputs);
            assert_eq!(read(&db, "form").await.inputs, saved.inputs);
        }
    }

    #[tokio::test]
    async fn the_write_needs_the_updated_at_it_read() {
        let db = database().await;
        let (board, pins) = form_board();
        insert(&db, &stale_event("form", &pins)).await;
        let read_at = row(&db, "form").await.updated_at;
        let mut event = read(&db, "form").await;
        event.inputs = event.inputs_from_board(&board).unwrap();

        let stale_read = read_at - chrono::Duration::seconds(1);
        assert!(
            !write_inputs_if_unchanged(&db, APP, &event, stale_read)
                .await
                .unwrap()
        );
        assert!(read(&db, "form").await.inputs_outdated());
        assert!(
            !write_inputs_if_unchanged(&db, "other-app", &event, read_at)
                .await
                .unwrap()
        );
        assert!(
            write_inputs_if_unchanged(&db, APP, &event, read_at)
                .await
                .unwrap()
        );
        assert_current(&read(&db, "form").await.inputs);
    }

    #[tokio::test]
    async fn without_a_board_node_or_live_row_the_stored_inputs_stay() {
        let db = database().await;
        let (board, pins) = form_board();
        let mut elsewhere = stale_event("elsewhere", &pins);
        elsewhere.board_id = "deleted-board".into();
        let mut orphan = stale_event("orphan", &pins);
        orphan.node_id = "deleted-node".into();
        for event in [&elsewhere, &orphan] {
            insert(&db, event).await;
        }
        let mut events = vec![read(&db, "elsewhere").await, read(&db, "orphan").await];
        refresh_outdated_inputs(
            &db,
            APP,
            &mut events,
            loader(board.clone(), &AtomicUsize::new(0)),
        )
        .await;
        assert_eq!(events[0].inputs, elsewhere.inputs);
        assert_eq!(events[1].inputs, orphan.inputs);
        assert!(read(&db, "elsewhere").await.inputs_outdated());
        assert!(read(&db, "orphan").await.inputs_outdated());

        insert(&db, &stale_event("live", &pins)).await;
        let mut archived = stale_event("live", &pins);
        archived.event_version = (0, 0, 1);
        let mut events = vec![archived];
        refresh_outdated_inputs(&db, APP, &mut events, loader(board, &AtomicUsize::new(0))).await;
        assert_current(&events[0].inputs);
        assert!(
            read(&db, "live").await.inputs_outdated(),
            "an archived version never writes the live row"
        );
    }
}
