use crate::event_kind::EventKind;
use anyhow::{Context, Result, ensure};
use flow_like_runtime::{
    app::{App, AppVisibility},
    flow::{board::Board, event::Event},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::FromProto;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc};

// A page leaves room for the management envelope inside a Noise message.
const PAGE_BYTES: usize = 10 * 1024;

pub async fn describe(
    state: &Path,
    project: &str,
    revision: &str,
    event_id: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    let root = crate::project_artifacts::managed_revision(state, project, revision)?;
    if let Some(id) = event_id {
        crate::config::validate_id("event", id)?;
    }
    if let Some(id) = after {
        crate::config::validate_id("cursor", id)?;
    }
    describe_snapshot(&root, project, revision, event_id, after).await
}

async fn describe_snapshot(
    root: &Path,
    project: &str,
    revision: &str,
    event_id: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    // Discovery needs metadata only. It never initializes node libraries, databases,
    // dependency downloads, or workflow execution in the management process.
    let mut config = FlowLikeConfig::new();
    let metadata =
        FlowLikeStore::Local(Arc::new(LocalObjectStore::new(root.to_path_buf())?)).read_only();
    config.register_app_meta_store(metadata.clone());
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    let app = App::load(project.to_owned(), state).await?;
    ensure!(
        app.id == project && matches!(app.visibility, AppVisibility::Offline),
        "Discovery requires the matching offline snapshot"
    );
    ensure!(
        app.events.len() <= 512,
        "Project event inventory exceeds discovery limit"
    );
    let mut rows = BTreeMap::new();
    if let Some(id) = event_id {
        ensure!(
            app.events.iter().any(|value| value == id),
            "Event is outside the project inventory"
        );
        let event = pinned_event(&app, id).await?;
        crate::config::validate_id("board", &event.board_id)?;
        let version = event
            .board_version
            .context("Event must pin a board version")?;
        ensure!(concrete(version), "Board version must be concrete");
        let board = Board::from_proto(
            Board::load_proto(
                metadata.as_generic(),
                &flow_like_storage::Path::from("apps").join(project),
                &event.board_id,
                Some(version),
            )
            .await?,
        );
        ensure!(
            board.id == event.board_id && board.version == version,
            "Board pin differs"
        );
        for variable in board.variables.values().chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        ) {
            if !variable.exposed && !variable.runtime_configured {
                continue;
            }
            crate::config::validate_id("variable", &variable.id)?;
            // Never serialize defaults, schemas, or event overrides. They may contain credentials.
            let row = json!({"id": variable.id, "name": variable.name.chars().take(120).collect::<String>(), "data_type": variable.data_type, "value_type": variable.value_type, "secret": variable.secret});
            if let Some(old) = rows.insert(variable.id.clone(), row.clone()) {
                ensure!(old == row, "Conflicting variable definitions");
            }
            ensure!(rows.len() <= 1024, "Too many configurable variables");
        }
    } else {
        let mut ids = app.events.clone();
        ids.sort();
        ids.dedup();
        // Read a bounded number of events per request, including unsupported entries.
        for id in ids
            .iter()
            .filter(|id| after.is_none_or(|cursor| id.as_str() > cursor))
            .take(8)
        {
            rows.insert(id.clone(), event_row(&app, &metadata, project, id).await?);
        }
        let (items, last) = page(&rows, None)?;
        let more = last
            .as_ref()
            .is_some_and(|last| ids.iter().any(|id| id > last));
        return Ok(
            json!({"project_id":project,"revision":revision,"event_id":null,"items":items,"next":if more { last } else { None }}),
        );
    }
    let (items, last) = page(&rows, after)?;
    let more = last
        .as_ref()
        .is_some_and(|last| rows.keys().any(|id| id > last));
    Ok(
        json!({"project_id":project,"revision":revision,"event_id":event_id,"items":items,"next":if more { last } else { None }}),
    )
}

/// How an event tells the supervisor that it runs. Clients read a closed set here, so a
/// schedule is `explicit` and `kind` says what it is.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Readiness {
    Listener,
    Explicit,
    Unsupported,
}

/// Active, without traffic variants, and pinned to concrete versions.
fn pinnable(event: &Event) -> bool {
    event.active
        && event.variant_set().is_empty()
        && event.board_version.is_some_and(concrete)
        && concrete(event.event_version)
}

/// Whether a device can run an event, and its reason when it cannot.
struct Verdict {
    eligible: bool,
    readiness: Readiness,
    /// The device's sentence for an event it cannot run.
    error: Option<String>,
    /// Why a device does not run the event, as the code clients word themselves.
    code: Option<String>,
    /// Facts of the row by their key: `route`, `schedule` or `once`.
    facts: Vec<(&'static str, Value)>,
}

impl Verdict {
    fn new(eligible: bool, readiness: Readiness) -> Self {
        Self {
            eligible,
            readiness,
            error: None,
            code: None,
            facts: Vec::new(),
        }
    }

    fn refused(self, sentence: String, code: &str) -> Self {
        Self {
            eligible: false,
            error: Some(sentence),
            code: Some(code.to_owned()),
            ..self
        }
    }

    /// A safe update needs an event whose readiness the supervisor can wait for.
    fn rollout_supported(&self) -> bool {
        self.eligible && self.readiness != Readiness::Unsupported
    }

    /// The listener serves it; an Endpoint's route must pass the route rule.
    fn served(event: &Event, pinned: bool) -> Self {
        let verdict = Self::new(pinned, Readiness::Listener);
        match crate::hosting::event_route(event) {
            Ok(None) => verdict,
            Ok(Some(route)) => Self {
                facts: vec![(
                    "route",
                    json!({"method": route.route.method, "path": route.route.path}),
                )],
                ..verdict
            },
            Err(problem) => verdict.refused(problem.to_string(), problem.code()),
        }
    }

    /// A schedule has no readiness node, so its flow is not loaded. A one-time schedule says
    /// when it runs and has no `schedule` key, which a reader of repeating ones would misread.
    fn scheduled(event: &Event, pinned: bool) -> Self {
        let verdict = Self::new(pinned, Readiness::Explicit);
        match crate::schedule::ScheduleSpec::from_event(event) {
            Ok(spec) => Self {
                facts: vec![match spec.once() {
                    Some(once) => ("once", json!(once)),
                    None => (
                        "schedule",
                        json!({"expression": spec.expression(), "timezone": spec.timezone()}),
                    ),
                }],
                ..verdict
            },
            Err(problem) => verdict.refused(problem.to_string(), problem.code()),
        }
    }

    /// A bot's settings must pass the settings rule; its token is checked when a deploy is
    /// validated, since discovery never reads secrets.
    #[cfg(feature = "bots")]
    fn bot(event: &Event, pinned: bool) -> Self {
        let verdict = Self::new(pinned, Readiness::Explicit);
        match flow_like_bots::BotSpec::from_config(&event.id, &event.event_type, &event.config) {
            Ok(_) => verdict,
            Err(problem) => verdict.refused(problem.to_string(), problem.code()),
        }
    }

    /// A service of its own process is ready when the one readiness node of its pinned
    /// flow says so.
    async fn service(metadata: &FlowLikeStore, project: &str, event: &Event) -> Result<Self> {
        use flow_like_runtime::flow::execution::service::ServiceReadyKind;
        let version = event.board_version.context("Missing board pin")?;
        let board = Board::from_proto(
            Board::load_proto(
                metadata.as_generic(),
                &flow_like_storage::Path::from("apps").join(project),
                &event.board_id,
                Some(version),
            )
            .await?,
        );
        Ok(
            match crate::runtime::service_readiness_source(event, &board) {
                Ok(Some((_, ServiceReadyKind::Daemon))) => Self::new(true, Readiness::Explicit),
                Ok(Some(_)) => Self::new(true, Readiness::Listener),
                Ok(None) => Self::new(true, Readiness::Unsupported),
                Err(error) => Self {
                    error: Some(error.to_string()),
                    ..Self::new(false, Readiness::Unsupported)
                },
            },
        )
    }
}

async fn listed_event(app: &App, id: &str) -> Result<Event> {
    crate::config::validate_id("event", id)?;
    let event = app.get_event(id, None).await?;
    ensure!(event.id == id, "Event identity differs");
    Ok(event)
}

/// One event as a deployer sees it: whether a device can run it, as which kind, and the
/// device's reason when it cannot.
async fn event_row(app: &App, metadata: &FlowLikeStore, project: &str, id: &str) -> Result<Value> {
    let event = listed_event(app, id).await?;
    // A type whose part this build lacks is answered as before the part existed.
    let kind = EventKind::runnable(&event.event_type, event.default_page_id.is_some());
    let pinned = kind.is_some() && pinnable(&event) && pinned_event(app, id).await.is_ok();
    let verdict = match kind {
        Some(EventKind::Served) => Verdict::served(&event, pinned),
        Some(EventKind::Scheduled) => Verdict::scheduled(&event, pinned),
        Some(EventKind::OnDemand) => Verdict::new(pinned, Readiness::Explicit),
        #[cfg(feature = "bots")]
        Some(EventKind::Bot) => Verdict::bot(&event, pinned),
        Some(EventKind::OwnServer | EventKind::Background) if pinned => {
            Verdict::service(metadata, project, &event).await?
        }
        _ => Verdict::new(false, Readiness::Unsupported),
    };
    let mut row = json!({"id": id, "name": event.name.chars().take(120).collect::<String>(), "event_type": event.event_type, "event_version": concrete(event.event_version).then_some(event.event_version), "board_version": event.board_version.filter(|version| concrete(*version)), "hosted": kind.is_some_and(EventKind::hosted), "eligible": verdict.eligible, "readiness_kind": verdict.readiness, "rollout_supported": verdict.rollout_supported(), "readiness_error": verdict.error, "kind": kind, "ineligible_code": verdict.code});
    for (key, fact) in verdict.facts {
        row[key] = fact;
    }
    Ok(row)
}

fn concrete(version: (u32, u32, u32)) -> bool {
    ![version.0, version.1, version.2].contains(&u32::MAX)
}
async fn pinned_event(app: &App, id: &str) -> Result<Event> {
    let live = app.get_event(id, None).await?;
    ensure!(
        live.id == id && concrete(live.event_version),
        "Event has no concrete version"
    );
    let archived = Event::load_pinned(id, app, live.event_version).await?;
    let mut archived_projection = serde_json::to_value(&archived)?;
    let mut live_projection = serde_json::to_value(&live)?;
    for projection in [&mut archived_projection, &mut live_projection] {
        let fields = projection
            .as_object_mut()
            .context("Invalid event metadata")?;
        fields.remove("created_at");
        fields.remove("updated_at");
    }
    ensure!(
        archived_projection == live_projection
            && archived.id == live.id
            && archived.event_version == live.event_version
            && archived.board_id == live.board_id
            && archived.board_version == live.board_version,
        "Pinned event content differs from the current event"
    );
    Ok(archived)
}
fn page(
    rows: &BTreeMap<String, Value>,
    after: Option<&str>,
) -> Result<(Vec<Value>, Option<String>)> {
    let mut items = Vec::new();
    let mut size = 0;
    let mut last = None;
    for (id, row) in rows
        .iter()
        .filter(|(id, _)| after.is_none_or(|cursor| id.as_str() > cursor))
    {
        let bytes = serde_json::to_vec(row)?.len();
        ensure!(bytes <= PAGE_BYTES, "Discovery entry exceeds page limit");
        if size + bytes > PAGE_BYTES {
            break;
        }
        size += bytes;
        items.push(row.clone());
        last = Some(id.clone());
    }
    Ok((items, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_bounded_ordered_and_make_progress() {
        let rows = (0..100)
            .map(|i| {
                (
                    format!("variable-{i:03}"),
                    json!({"id": format!("variable-{i:03}"), "name":"x".repeat(120)}),
                )
            })
            .collect();
        let (first, cursor) = page(&rows, None).unwrap();
        assert!(!first.is_empty() && first.len() < 100);
        let (second, _) = page(&rows, cursor.as_deref()).unwrap();
        assert_eq!(first.len() + second.len(), 100);
        assert!(serde_json::to_vec(&first).unwrap().len() < 12 * 1024);
        assert!(second[0]["id"].as_str().unwrap() > cursor.as_deref().unwrap());
    }

    type Change = fn(&mut Event);

    fn schedule(event: &mut Event, config: Value) {
        event.event_type = "cron".into();
        event.config = serde_json::to_vec(&config).unwrap();
    }

    /// Discovery rows of the runtime fixture's project with one event per entry, each a
    /// changed copy of its daemon event. Events named in `archived` also have the archive
    /// of their event version, as a staged copy has.
    async fn discovered(events: &[(&str, Change)], archived: &[&str]) -> BTreeMap<String, Value> {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, _, _) = crate::runtime::tests::fixture(directory.path()).await;
        let mut app = App::load(config.project_id.clone(), state).await.unwrap();
        let base = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        for (id, change) in events {
            let mut event = base.clone();
            event.id = (*id).to_owned();
            change(&mut event);
            event.save(&app, None).await.unwrap();
            if archived.contains(id) {
                event.save(&app, Some(event.event_version)).await.unwrap();
            }
        }
        app.events = events.iter().map(|(id, _)| (*id).to_owned()).collect();
        app.save().await.unwrap();
        let mut rows = BTreeMap::new();
        let mut after: Option<String> = None;
        loop {
            let page = describe_snapshot(
                directory.path(),
                "project",
                "revision",
                None,
                after.as_deref(),
            )
            .await
            .unwrap();
            for row in page["items"].as_array().unwrap() {
                rows.insert(row["id"].as_str().unwrap().to_owned(), row.clone());
            }
            after = page["next"].as_str().map(str::to_owned);
            if after.is_none() {
                return rows;
            }
        }
    }

    fn nightly(event: &mut Event) {
        schedule(
            event,
            json!({"expression": "0 0 2 * * *", "timezone": "Europe/Berlin"}),
        )
    }

    /// Half of the event's triggers go to a second copy of its flow.
    fn split_traffic(event: &mut Event) {
        event.canary = Some(flow_like_runtime::flow::event::CanaryEvent {
            weight: 0.5,
            variables: Default::default(),
            board_id: event.board_id.clone(),
            board_version: event.board_version,
            node_id: event.node_id.clone(),
            created_at: event.created_at,
            updated_at: event.updated_at,
        });
    }

    #[tokio::test]
    async fn a_repeating_schedule_is_eligible_unless_it_has_a_page_is_paused_or_split() {
        let rows = discovered(
            &[
                ("report", nightly),
                ("weekdays", |event| {
                    schedule(event, json!({"cron": " 0  9 * * mon-fri "}))
                }),
                ("page", |event| {
                    nightly(event);
                    event.default_page_id = Some("page".into());
                }),
                ("paused", |event| {
                    nightly(event);
                    event.active = false;
                }),
                ("variant", |event| {
                    nightly(event);
                    split_traffic(event);
                }),
            ],
            &[],
        )
        .await;
        // The literal row of the design, with this fixture's name and versions.
        assert_eq!(
            rows["report"],
            json!({"id":"report","name":"Persistent service","event_type":"cron","event_version":[1,0,0],"board_version":[1,0,0],
                "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
                "kind":"scheduled","schedule":{"expression":"0 0 2 * * *","timezone":"Europe/Berlin"},"ineligible_code":null})
        );
        assert_eq!(
            rows["weekdays"]["schedule"],
            json!({"expression": "0 9 * * MON-FRI", "timezone": "UTC"})
        );
        // A `cron` event with a default Page is today's served row plus `kind`.
        assert_eq!(
            rows["page"],
            json!({"id":"page","name":"Persistent service","event_type":"cron","event_version":[1,0,0],"board_version":[1,0,0],
                "hosted":true,"eligible":true,"readiness_kind":"listener","rollout_supported":true,"readiness_error":null,
                "kind":"served","ineligible_code":null})
        );
        // Paused events and traffic variants stay out, whatever their schedule.
        for id in ["paused", "variant"] {
            let row = &rows[id];
            assert_eq!(row["eligible"], false, "{id}");
            assert_eq!(row["rollout_supported"], false, "{id}");
            assert_eq!(row["kind"], "scheduled", "{id}");
            assert!(row["schedule"].is_object() && row["ineligible_code"].is_null());
        }
    }

    #[tokio::test]
    async fn every_row_says_its_kind() {
        let rows = discovered(
            &[
                ("background", |_| ()),
                ("chat", |event| event.event_type = "simple_chat".into()),
                ("mail", |event| event.event_type = "email".into()),
                ("own-server", |event| event.event_type = "rest".into()),
                ("report", nightly),
            ],
            &[],
        )
        .await;
        let facts: Vec<_> = rows
            .iter()
            .map(|(id, row)| {
                let (kind, readiness) = (&row["kind"], &row["readiness_kind"]);
                json!([id, kind, readiness, row["eligible"]])
            })
            .collect();
        assert_eq!(
            json!(facts),
            json!([
                ["background", "background", "explicit", true],
                ["chat", "served", "listener", true],
                ["mail", null, "unsupported", false],
                // Its flow has no REST server node to report readiness.
                ["own-server", "own_server", "unsupported", false],
                ["report", "scheduled", "explicit", true],
            ])
        );
        assert!(rows["own-server"]["readiness_error"].is_string());
        assert!(rows.values().all(|row| row["ineligible_code"].is_null()));
        assert!(rows.values().all(|row| row.get("route").is_none()));
    }

    fn endpoint(event: &mut Event, config: Value) {
        event.event_type = "api".into();
        event.config = serde_json::to_vec(&config).unwrap();
    }

    fn web_request(event: &mut Event, config: Value) {
        event.event_type = "http".into();
        event.config = serde_json::to_vec(&config).unwrap();
    }

    /// The row of an event this build cannot run: what every agent answered before the part
    /// that runs its type existed.
    fn left_out(id: &str, event_type: &str) -> Value {
        json!({"id":id,"name":"Persistent service","event_type":event_type,"event_version":[1,0,0],"board_version":[1,0,0],
            "hosted":false,"eligible":false,"readiness_kind":"unsupported","rollout_supported":false,"readiness_error":null,
            "kind":null,"ineligible_code":null})
    }

    #[tokio::test]
    async fn an_endpoint_says_its_route_and_a_build_without_endpoints_answers_as_before() {
        let rows = discovered(
            &[
                ("orders", |event| {
                    endpoint(
                        event,
                        json!({"sink_type":"http","method":"GET","path":"/orders","public_endpoint":false}),
                    )
                }),
                ("page", |event| {
                    endpoint(event, json!({"method":"GET","path":"/services"}));
                    event.default_page_id = Some("page".into());
                }),
                ("lenient", |event| {
                    web_request(event, json!({"path":"orders"}))
                }),
            ],
            &[],
        )
        .await;
        // The literal row of design §1.5, with this fixture's name and versions.
        let served = json!({"id":"orders","name":"Persistent service","event_type":"api","event_version":[1,0,0],"board_version":[1,0,0],
            "hosted":true,"eligible":true,"readiness_kind":"listener","rollout_supported":true,"readiness_error":null,
            "kind":"served","route":{"method":"GET","path":"/orders"},"ineligible_code":null});
        if cfg!(feature = "api-events") {
            assert_eq!(rows["orders"], served);
        } else {
            assert_eq!(rows["orders"], left_out("orders", "api"));
        }
        // With a default Page it is a Page, whatever its route says.
        assert_eq!(rows["page"]["kind"], "served");
        assert_eq!(rows["page"]["eligible"], true);
        assert!(rows["page"].get("route").is_none());
        // Read as the hub reads it: a leading `/` is added and the method is POST.
        let lenient = &rows["lenient"];
        assert_eq!(lenient["route"], json!({"method":"POST","path":"/orders"}));
        assert_eq!(lenient["eligible"], true);
    }

    #[tokio::test]
    async fn each_route_a_device_cannot_serve_says_why() {
        let rows = discovered(
            &[
                ("invalid", |event| {
                    web_request(event, json!({"path":"/a b"}))
                }),
                ("method", |event| {
                    web_request(event, json!({"path":"/x","method":"TRACE"}))
                }),
                ("missing", |event| web_request(event, json!({"path":7}))),
                ("reserved", |event| {
                    web_request(event, json!({"path":"/channels/1","method":"POST"}))
                }),
            ],
            &[],
        )
        .await;
        for (id, code) in [
            ("invalid", "route_invalid"),
            ("method", "route_invalid"),
            ("missing", "route_missing"),
            ("reserved", "route_reserved"),
        ] {
            let row = &rows[id];
            assert_eq!(row["ineligible_code"], code, "{id}");
            assert_eq!(row["eligible"], false, "{id}");
            assert_eq!(row["rollout_supported"], false, "{id}");
            assert_eq!(
                (&row["kind"], &row["hosted"], &row["readiness_kind"]),
                (&json!("served"), &json!(true), &json!("listener")),
                "{id}"
            );
            assert!(row.get("route").is_none(), "{id}");
            let sentence = row["readiness_error"].as_str().unwrap();
            assert!((20..=480).contains(&sentence.len()), "{sentence}");
        }
        assert!(
            rows["method"]["readiness_error"]
                .as_str()
                .unwrap()
                .contains("TRACE")
        );
    }

    #[tokio::test]
    async fn a_form_is_started_by_a_person_and_a_build_without_forms_answers_as_before() {
        let rows = discovered(
            &[
                ("action", |event| event.event_type = "quick_action".into()),
                ("notes", |event| {
                    event.event_type = "generic_form".into();
                    event.config = br#"{"navigate_to_routes":["/notes"]}"#.to_vec();
                }),
            ],
            &[],
        )
        .await;
        // The literal row of design §1.5, with this fixture's name and versions.
        let started = |id: &str, event_type: &str| {
            json!({"id":id,"name":"Persistent service","event_type":event_type,"event_version":[1,0,0],"board_version":[1,0,0],
                "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
                "kind":"on_demand","ineligible_code":null})
        };
        for (id, event_type) in [("action", "quick_action"), ("notes", "generic_form")] {
            let expected = if cfg!(feature = "on-demand") {
                started(id, event_type)
            } else {
                left_out(id, event_type)
            };
            assert_eq!(rows[id], expected, "{id}");
        }
    }

    fn bot(event: &mut Event, event_type: &str, config: Value) {
        event.event_type = event_type.into();
        event.config = serde_json::to_vec(&config).unwrap();
    }

    #[tokio::test]
    async fn a_bot_says_when_its_settings_are_unreadable_and_a_build_without_bots_answers_as_before()
     {
        let rows = discovered(
            &[
                ("helper", |event| {
                    bot(
                        event,
                        "telegram",
                        json!({"bot_name":"helper_bot","chat_whitelist":["12345"]}),
                    )
                }),
                ("listed", |event| {
                    bot(event, "telegram", json!({"chat_whitelist":"12345"}))
                }),
                ("server", |event| {
                    bot(event, "discord", json!({"intents":["Guilds"]}))
                }),
                ("unknown", |event| {
                    bot(event, "discord", json!({"intents":["Telepathy"]}))
                }),
            ],
            &[],
        )
        .await;
        let telegram = cfg!(feature = "bots-telegram");
        assert_bot_rows(&rows, telegram, "telegram", ["helper", "listed"]);
        let discord = cfg!(feature = "bots-discord");
        assert_bot_rows(&rows, discord, "discord", ["server", "unknown"]);
    }

    /// A bot whose settings pass and one whose settings do not, as a build with or without
    /// the provider's part answers them.
    fn assert_bot_rows(
        rows: &BTreeMap<String, Value>,
        built: bool,
        event_type: &str,
        [valid, invalid]: [&str; 2],
    ) {
        if !built {
            for id in [valid, invalid] {
                assert_eq!(rows[id], left_out(id, event_type), "{id}");
            }
            return;
        }
        // The literal row of design §1.5, with this fixture's name and versions.
        assert_eq!(
            rows[valid],
            json!({"id":valid,"name":"Persistent service","event_type":event_type,"event_version":[1,0,0],"board_version":[1,0,0],
                "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
                "kind":"bot","ineligible_code":null})
        );
        let row = &rows[invalid];
        assert_eq!(
            (&row["kind"], &row["eligible"], &row["rollout_supported"]),
            (&json!("bot"), &json!(false), &json!(false)),
            "{invalid}"
        );
        assert_eq!(row["ineligible_code"], "bot_invalid", "{invalid}");
        let sentence = row["readiness_error"].as_str().unwrap();
        assert!((20..=480).contains(&sentence.len()), "{sentence}");
    }

    #[tokio::test]
    async fn each_schedule_a_device_does_not_run_says_why() {
        let rows = discovered(
            &[
                ("invalid", |event| {
                    schedule(event, json!({"expression": "0 0 9 ? * *"}))
                }),
                ("missing", |event| schedule(event, json!({}))),
                ("often", |event| {
                    schedule(event, json!({"expression": "*/30 * * * * *"}))
                }),
                ("zone", |event| {
                    schedule(
                        event,
                        json!({"expression": "0 9 * * *", "timezone": "Mars/Olympus"}),
                    )
                }),
            ],
            &[],
        )
        .await;
        for (id, code) in [
            ("invalid", "schedule_invalid"),
            ("missing", "schedule_missing"),
            ("often", "schedule_too_often"),
            ("zone", "schedule_invalid"),
        ] {
            let row = &rows[id];
            assert_eq!(row["ineligible_code"], code, "{id}");
            assert_eq!(row["eligible"], false, "{id}");
            assert_eq!(row["rollout_supported"], false, "{id}");
            assert_eq!(row["kind"], "scheduled", "{id}");
            assert!(row.get("schedule").is_none(), "{id}");
            let sentence = row["readiness_error"].as_str().unwrap();
            assert!((20..=480).contains(&sentence.len()), "{sentence}");
        }
        assert!(
            rows["zone"]["readiness_error"]
                .as_str()
                .unwrap()
                .contains("Mars/Olympus")
        );
    }

    fn once(event: &mut Event, date: &str, time: &str) {
        schedule(
            event,
            json!({"scheduled_for": {"date": date, "time": time}, "timezone": "Europe/Berlin"}),
        )
    }

    #[tokio::test]
    async fn a_one_time_schedule_says_when_it_runs_and_a_build_without_them_answers_as_before() {
        let rows = discovered(
            &[
                ("both", |event| {
                    schedule(
                        event,
                        json!({"expression": "0 9 * * *", "scheduled_for": {"date": "2026-12-24", "time": "18:00"}}),
                    )
                }),
                ("gap", |event| once(event, "2027-03-28", "02:30")),
                ("migration", |event| once(event, "2026-09-24", "09:00")),
                ("past", |event| once(event, "2001-01-01", "09:00")),
            ],
            &[],
        )
        .await;
        // A repeating expression beside a one-time date is refused in every build.
        assert_eq!(rows["both"]["ineligible_code"], "schedule_invalid");
        assert!(rows["both"].get("schedule").is_none() && rows["both"].get("once").is_none());
        if !cfg!(feature = "scheduled-once") {
            for id in ["gap", "migration", "past"] {
                let row = &rows[id];
                assert_eq!(row["ineligible_code"], "schedule_once", "{id}");
                assert_eq!(row["eligible"], false, "{id}");
                assert!(row.get("schedule").is_none() && row.get("once").is_none());
            }
            return;
        }
        // The literal row of design §1.5, with this fixture's name and versions.
        assert_eq!(
            rows["migration"],
            json!({"id":"migration","name":"Persistent service","event_type":"cron","event_version":[1,0,0],"board_version":[1,0,0],
                "hosted":false,"eligible":true,"readiness_kind":"explicit","rollout_supported":true,"readiness_error":null,
                "kind":"scheduled","once":{"date":"2026-09-24","time":"09:00","at":1790233200,"timezone":"Europe/Berlin"},"ineligible_code":null})
        );
        // Whether its time has passed is the device's state, never a reason in discovery.
        assert_eq!(rows["past"]["eligible"], true);
        assert_eq!(rows["past"]["once"]["date"], "2001-01-01");
        // A local time that does not exist is refused, with no time of its own.
        let gap = &rows["gap"];
        assert_eq!(gap["ineligible_code"], "schedule_invalid");
        assert!(gap.get("once").is_none() && gap.get("schedule").is_none());
    }

    #[tokio::test]
    async fn an_event_the_export_pinned_from_latest_is_eligible() {
        let nightly: Change = |event| {
            schedule(event, json!({"expression": "0 0 2 * * *"}));
            event.event_version = (1, 0, 3);
        };
        // The app's own record follows the latest flow: a device cannot run that.
        let latest = discovered(
            &[("report", |event| {
                schedule(event, json!({"expression": "0 0 2 * * *"}));
                event.event_version = (1, 0, 3);
                event.board_version = None;
            })],
            &[],
        )
        .await;
        assert_eq!(latest["report"]["eligible"], false);
        assert!(latest["report"]["board_version"].is_null());
        // In the staged copy the export rewrote the live document and the archive of the
        // same event version to the published flow version.
        let pinned = discovered(&[("report", nightly)], &["report"]).await;
        assert_eq!(pinned["report"]["eligible"], true);
        assert_eq!(pinned["report"]["event_version"], json!([1, 0, 3]));
        assert_eq!(pinned["report"]["board_version"], json!([1, 0, 0]));
        assert_eq!(pinned["report"]["kind"], "scheduled");
    }
}
