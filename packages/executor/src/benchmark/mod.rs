//! Local widget-action benchmark. Enabled only with the `benchmarks` feature.
//!
//! Production execution, accounting, artifact loading, and widget lookup run
//! against synthetic data and a loopback hub. Delays model excluded cloud work.

mod workload;

use crate::{
    execute::resolve_run_template, jwt::ExecutorClaims, quota, types::ExecutionRequest,
    widgets::HubAccess,
};
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use flow_like::{
    flow::compiled::{append_widget_snapshot, ArtifactWidgetEntry, ArtifactWidgetSnapshot},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_types::{
    anyhow,
    intercom::{InterComCallback, InterComEvent},
    Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::{mpsc, Semaphore};
use workload::{Workload, APP_ID, BOARD_ID, PANEL_ID, START_ID};

#[derive(Clone, Debug, Serialize)]
struct Options {
    port: u16,
    slots: usize,
    persons: usize,
    rows: usize,
    dormant_nodes: usize,
    inventory: usize,
    root: PathBuf,
    profile: String,
    dispatch_ms: u64,
    setup_ms: u64,
    compute_start_ms: u64,
    admission_ms: u64,
    widget_ms: u64,
    artifact_ms: u64,
    bundle_validation_ms: u64,
    durability_ms: u64,
    finish_ms: u64,
    compute_completed_ms: u64,
}

impl Options {
    fn parse() -> Result<Option<Self>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--help") {
            println!("widget_action_bench [--profile local|calibrated] [--port 4340] [--slots 4] [--persons 100] [--rows 12] [--inventory 16] [--dormant-nodes 600] [--root PATH]\nDelay overrides (milliseconds): --dispatch-ms --setup-ms --compute-start-ms --admission-ms --widget-ms --artifact-ms --bundle-validation-ms --durability-ms --finish-ms --compute-completed-ms\nLocal has no injected delays. Calibrated is an illustrative transport model, not an AWS latency measurement. The server binds only to 127.0.0.1.");
            return Ok(None);
        }
        if args.len() % 2 != 0 {
            return Err(anyhow!("options require a value; use --help"));
        }
        let profile = args
            .chunks(2)
            .find(|pair| pair[0] == "--profile")
            .map(|pair| pair[1].as_str())
            .unwrap_or("local");
        if !matches!(profile, "local" | "calibrated") {
            return Err(anyhow!("profile must be local or calibrated"));
        }
        let mut value = Self {
            port: 4340,
            slots: 4,
            persons: 100,
            rows: 12,
            dormant_nodes: 600,
            inventory: 16,
            root: std::env::temp_dir().join("flow-widget-action-bench"),
            profile: profile.into(),
            dispatch_ms: 0,
            setup_ms: 0,
            compute_start_ms: 0,
            admission_ms: 0,
            widget_ms: 0,
            artifact_ms: 0,
            bundle_validation_ms: 0,
            durability_ms: 0,
            finish_ms: 0,
            compute_completed_ms: 0,
        };
        if profile == "calibrated" {
            value.dispatch_ms = 350;
            value.setup_ms = 100;
            value.compute_start_ms = 150;
            value.admission_ms = 55;
            value.widget_ms = 285;
            value.artifact_ms = 20;
            value.bundle_validation_ms = 20;
            value.durability_ms = 300;
            value.finish_ms = 200;
            value.compute_completed_ms = 80;
        }
        for pair in args.chunks(2) {
            let raw = &pair[1];
            match pair[0].as_str() {
                "--profile" => (),
                "--root" => value.root = raw.into(),
                "--port" => value.port = raw.parse()?,
                "--slots" => value.slots = raw.parse()?,
                "--persons" => value.persons = raw.parse()?,
                "--rows" => value.rows = raw.parse()?,
                "--inventory" => value.inventory = raw.parse()?,
                "--dormant-nodes" => value.dormant_nodes = raw.parse()?,
                "--dispatch-ms" => value.dispatch_ms = raw.parse()?,
                "--setup-ms" => value.setup_ms = raw.parse()?,
                "--compute-start-ms" => value.compute_start_ms = raw.parse()?,
                "--admission-ms" => value.admission_ms = raw.parse()?,
                "--widget-ms" => value.widget_ms = raw.parse()?,
                "--artifact-ms" => value.artifact_ms = raw.parse()?,
                "--bundle-validation-ms" => value.bundle_validation_ms = raw.parse()?,
                "--durability-ms" => value.durability_ms = raw.parse()?,
                "--finish-ms" => value.finish_ms = raw.parse()?,
                "--compute-completed-ms" => value.compute_completed_ms = raw.parse()?,
                other => return Err(anyhow!("unknown option {other}")),
            }
        }
        if value.port == 0
            || !(1..=256).contains(&value.slots)
            || !(1..=1024).contains(&value.inventory)
            || value.persons < 2
        {
            return Err(anyhow!(
                "port must be positive, slots 1..256, inventory 1..1024, persons at least 2"
            ));
        }
        if [
            value.dispatch_ms,
            value.setup_ms,
            value.compute_start_ms,
            value.admission_ms,
            value.widget_ms,
            value.artifact_ms,
            value.bundle_validation_ms,
            value.durability_ms,
            value.finish_ms,
            value.compute_completed_ms,
        ]
        .into_iter()
        .any(|ms| ms > 5_000)
        {
            return Err(anyhow!("each injected delay must be at most 5000ms"));
        }
        Ok(Some(value))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Variant {
    Baseline,
    Overlap,
    Bundled,
    Optimized,
}
impl Variant {
    fn bundled(self) -> bool {
        matches!(self, Self::Bundled | Self::Optimized)
    }
    fn overlap(self) -> bool {
        matches!(self, Self::Overlap | Self::Optimized)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Cache {
    Warm,
    Cold,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    variant: Variant,
    cache: Cache,
    person_id: usize,
}

#[derive(Default, Serialize)]
struct Measurements {
    queue_ms: f64,
    prepare_ms: f64,
    admission_ms: f64,
    graph_ms: f64,
    history_patch_ms: Option<f64>,
    terminal_ms: f64,
    widget_gets: usize,
    artifact_gets: usize,
    callback_requests: usize,
    rows: usize,
    verified: bool,
    spans: Vec<Value>,
    node_traces: Vec<Value>,
}
struct Trial {
    origin: Instant,
    wall_origin: SystemTime,
    action: Action,
    measurements: Mutex<Measurements>,
}
impl Trial {
    fn span(&self, name: &str, start: Instant) {
        self.measurements.lock().unwrap().spans.push(json!({"name":name,"start_ms":ms(start.duration_since(self.origin)),"end_ms":ms(self.origin.elapsed())}));
    }
    async fn delay(&self, name: &str, millis: u64) {
        let start = Instant::now();
        if millis > 0 {
            tokio::time::sleep(Duration::from_millis(millis)).await;
        }
        self.span(name, start);
    }
}
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

struct Bench {
    options: Options,
    workload: Workload,
    bundle: Vec<u8>,
    inventory: Vec<flow_like::a2ui::widget::Widget>,
    slots: Arc<Semaphore>,
    trials: Mutex<HashMap<String, Arc<Trial>>>,
    session: String,
}
impl Bench {
    fn trial(&self, id: &str) -> std::result::Result<Arc<Trial>, StatusCode> {
        self.trials
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(StatusCode::NOT_FOUND)
    }
}

pub async fn serve() -> Result<()> {
    let Some(options) = Options::parse()? else {
        return Ok(());
    };
    let workload = Workload::prepare(
        &options.root,
        options.persons,
        options.rows,
        options.dormant_nodes,
    )
    .await?;
    let bundle = append_widget_snapshot(
        &workload.artifact,
        &ArtifactWidgetSnapshot {
            format_version: 1,
            app_id: APP_ID.into(),
            board_id: BOARD_ID.into(),
            board_identity: "1_0_0".into(),
            revision: "synthetic-v1".into(),
            widgets: vec![ArtifactWidgetEntry {
                selector: workload.widget.name.clone(),
                widget_id: workload.widget.id.clone(),
                source_etag: "synthetic-widget-v1".into(),
                widget: workload.widget.clone(),
            }],
        },
    )?;
    let mut inventory = vec![workload.widget.clone()];
    for index in 1..options.inventory {
        let mut widget = workload.widget.clone();
        widget.id = format!("other-widget-{index}");
        widget.name = format!("Other widget {index}");
        inventory.push(widget);
    }
    let address = format!("127.0.0.1:{}", options.port);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    let state = Arc::new(Bench {
        slots: Arc::new(Semaphore::new(options.slots)),
        options,
        workload,
        bundle,
        inventory,
        trials: Mutex::new(HashMap::new()),
        session: flow_like_types::create_id(),
    });
    let router = Router::new()
        .route("/health", get(health))
        .route("/action", post(action))
        .route("/_bench/{id}/artifact", get(artifact))
        .route("/_bench/{id}/api/v1/execution/quota", post(accounting))
        .route(
            "/_bench/{id}/api/v1/execution/apps/{app}/widgets",
            get(widgets),
        )
        .with_state(state);
    println!("Widget action benchmark ready: http://{address}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn health(State(bench): State<Arc<Bench>>) -> Json<Value> {
    Json(
        json!({"schema_version":1,"options":bench.options,"profile":bench.options.profile,
        "build":{"debug_assertions":cfg!(debug_assertions),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"runtime":"tokio-current-thread"},
        "expected_rows":bench.workload.rows_per_person,"board_nodes":bench.workload.board_nodes,"executed_graph_nodes":3,
        "expected_widget_gets":{"baseline":1,"overlap":1,"bundled":0,"optimized":0},
        "limitations":["Synthetic DB-filter → Instantiate Widget → Push To Container; not the exported production graph.",
            "Cold resets the executor template cache key; process, connection pools and local database remain warm.",
            "API authentication, quota database, Lambda dispatch, snapshot producer HEAD reads and durable remote log storage are excluded.",
            "Configured delays stand in for excluded cloud work; calibrated settings are illustrative, not AWS percentiles.",
            "The backend history_patch_ms boundary is the semantic table patch, not browser paint. Dormant nodes exercise preparation, not the live app's loop.",
            "The local database is already open. Cold samples do not clear the OS page cache.",
            "Counters and spans stop at terminal delivery. Final compute accounting runs afterward and still occupies an executor slot."]}),
    )
}

async fn artifact(
    State(bench): State<Arc<Bench>>,
    Path(id): Path<String>,
) -> std::result::Result<Response, StatusCode> {
    let trial = bench.trial(&id)?;
    trial.measurements.lock().unwrap().artifact_gets += 1;
    trial
        .delay("artifact_http", bench.options.artifact_ms)
        .await;
    Ok(if trial.action.variant.bundled() {
        bench.bundle.clone()
    } else {
        bench.workload.artifact.clone()
    }
    .into_response())
}
async fn widgets(
    State(bench): State<Arc<Bench>>,
    Path((id, app)): Path<(String, String)>,
) -> std::result::Result<Json<Value>, StatusCode> {
    if app != APP_ID {
        return Err(StatusCode::FORBIDDEN);
    }
    let trial = bench.trial(&id)?;
    trial.measurements.lock().unwrap().widget_gets += 1;
    trial
        .delay("widget_inventory_http", bench.options.widget_ms)
        .await;
    Ok(Json(json!(bench.inventory)))
}
async fn accounting(
    State(bench): State<Arc<Bench>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> std::result::Result<Json<Value>, StatusCode> {
    let trial = bench.trial(&id)?;
    trial.measurements.lock().unwrap().callback_requests += 1;
    let (name, delay) = match (body["phase"].as_str(), body["compute"]["status"].as_str()) {
        (Some("attempt"), Some("started")) => {
            ("compute_started_http", bench.options.compute_start_ms)
        }
        (Some("attempt"), _) => ("compute_completed_http", bench.options.compute_completed_ms),
        (Some("start"), _) => ("admission_http", bench.options.admission_ms),
        (Some("finish"), _) => ("finish_http", bench.options.finish_ms),
        _ => ("other_callback_http", 0),
    };
    trial.delay(name, delay).await;
    Ok(Json(json!({"accepted":true})))
}

async fn action(State(bench): State<Arc<Bench>>, Json(action): Json<Action>) -> Response {
    if action.person_id == 0 || action.person_id > bench.workload.persons {
        return (
            StatusCode::BAD_REQUEST,
            "person_id outside synthetic dataset",
        )
            .into_response();
    }
    let id = flow_like_types::create_id();
    let trial = Arc::new(Trial {
        origin: Instant::now(),
        wall_origin: SystemTime::now(),
        action,
        measurements: Mutex::new(Measurements {
            rows: bench.workload.rows_per_person,
            ..Default::default()
        }),
    });
    bench
        .trials
        .lock()
        .unwrap()
        .insert(id.clone(), trial.clone());
    let (tx, rx) = mpsc::channel::<Bytes>(8);
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            run_action(&bench, &id, &trial, &tx),
        )
        .await;
        match result {
            Ok(Ok(())) => (),
            other => {
                let _ = send(&tx, json!({"type":"error","error":format!("{other:?}")})).await;
            }
        }
        bench.trials.lock().unwrap().remove(&id);
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|bytes| (Ok::<_, Infallible>(bytes), rx))
    });
    (
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn send(tx: &mpsc::Sender<Bytes>, value: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    tx.send(Bytes::from(bytes))
        .await
        .map_err(|_| anyhow!("benchmark client disconnected"))
}

async fn run_action(
    bench: &Arc<Bench>,
    id: &str,
    trial: &Arc<Trial>,
    tx: &mpsc::Sender<Bytes>,
) -> Result<()> {
    trial
        .delay("dispatch_delay", bench.options.dispatch_ms)
        .await;
    if trial.action.variant.bundled() {
        trial
            .delay(
                "bundle_validation_delay",
                bench.options.bundle_validation_ms,
            )
            .await;
    }
    let queue_start = Instant::now();
    let _slot = bench.slots.acquire().await?;
    trial.measurements.lock().unwrap().queue_ms = ms(queue_start.elapsed());
    trial.span("executor_queue", queue_start);
    let callback_url = format!("http://127.0.0.1:{}/_bench/{id}", bench.options.port);
    let claims: ExecutorClaims = serde_json::from_value(
        json!({"sub":"benchmark","run_id":id,"app_id":APP_ID,"board_id":BOARD_ID,
        "callback_url":callback_url,"runtime_limit_ms":60_000,"typ":"executor","iss":"benchmark","aud":"benchmark","iat":0,"nbf":0,"exp":9999999999_i64,"jti":id}),
    )?;
    let context = quota::ComputeContext::new("local-widget-benchmark".into(), id.into(), 2048);
    quota::CURRENT_COMPUTE.scope(context, async {
        let result = run_scoped(bench, id, trial, tx, &claims).await;
        if let Err(error) = result {
            quota::record_compute(&claims, "local-benchmark", "failed").await;
            return Err(error);
        }
        let sample = {
            let mut measurements = trial.measurements.lock().unwrap();
            measurements.terminal_ms = ms(trial.origin.elapsed());
            json!({"id":id,"variant":trial.action.variant,"cache":trial.action.cache,"person_id":trial.action.person_id,"metrics":*measurements})
        };
        // Keep stage fields directly accessible to report tools.
        let mut flat = sample["metrics"].as_object().unwrap().clone();
        for key in ["id","variant","cache","person_id"] { flat.insert(key.into(),sample[key].clone()); }
        let delivered = send(tx, json!({"type":"complete","sample":flat})).await;
        quota::record_compute(&claims, "local-benchmark", "completed").await;
        delivered
    }).await
}

async fn run_scoped(
    bench: &Arc<Bench>,
    id: &str,
    trial: &Arc<Trial>,
    tx: &mpsc::Sender<Bytes>,
    claims: &ExecutorClaims,
) -> Result<()> {
    let mut compute = if trial.action.variant.overlap() {
        quota::start_compute(claims, "local-benchmark").await
    } else {
        quota::record_compute(claims, "local-benchmark", "started").await;
        quota::ComputeStart::default()
    };
    let prepare_start = Instant::now();
    let setup = async {
        trial.delay("setup_delay", bench.options.setup_ms).await;
        let mut state = FlowLikeState::new(FlowLikeConfig::new(), HTTPClient::new_without_refetch());
        state.node_registry.write().await.node_registry = bench.workload.registry.clone();
        HubAccess { callback_url: claims.callback_url.clone(), jwt: "local-benchmark".into(), hosted_frontend: false }.register_on(&mut state).await;
        let state = Arc::new(state);
        let revision = match trial.action.cache { Cache::Warm => bench.session.as_str(), Cache::Cold => id };
        let request: ExecutionRequest = serde_json::from_value(json!({
            "app_id":APP_ID,"board_id":BOARD_ID,"board_version":[1,0,0],"node_id":START_ID,"executor_jwt":"local-benchmark",
            "credentials":{"Aws":{"access_key_id":"benchmark-unused","secret_access_key":"benchmark-unused","session_token":null,"meta_bucket":"unused","content_bucket":"unused","logs_bucket":"unused","region":"local","expiration":null}},
            "artifact":{"url":format!("{}/artifact",claims.callback_url),"path":format!("benchmark/{revision}/{}",if trial.action.variant.bundled(){"bundled.flcb"}else{"legacy.flcb"}),"registry_fingerprint":blake3::Hash::from_bytes(bench.workload.registry.fingerprint()).to_hex().to_string()}
        }))?;
        let template = resolve_run_template(&state, &request).await?;
        let pending = Arc::new(Mutex::new(Vec::<Value>::new()));
        let (callback_trial, tx) = (trial.clone(), tx.clone());
        let callback: InterComCallback = Some(Arc::new(move |event: InterComEvent| {
            let (trial, tx, pending) = (callback_trial.clone(), tx.clone(), pending.clone());
            Box::pin(async move {
                if event.event_type == "a2ui" {
                    let ready = event.payload["type"] == "upsertElement" && event.payload["element_id"] == PANEL_ID && event.payload["value"]["type"] == "pushChild";
                    let batch = {
                        let mut messages = pending.lock().unwrap(); messages.push(event.payload);
                        ready.then(|| std::mem::take(&mut *messages))
                    };
                    if let Some(messages) = batch {
                        trial.measurements.lock().unwrap().history_patch_ms = Some(ms(trial.origin.elapsed()));
                        send(&tx, json!({"type":"patch","messages":messages,"person_id":trial.action.person_id,"expected_rows":bench_rows(&trial)})).await?;
                    }
                }
                Ok(())
            })
        }));
        let run = bench.workload.new_run(&state, template, callback, trial.action.person_id).await?;
        Ok::<_, flow_like_types::Error>((state, run))
    }.await;
    // A setup error must not leave an accounting request racing terminal evidence.
    compute.wait().await;
    let (state, mut run) = setup?;
    trial.span("prepare_with_accounting_join", prepare_start);
    trial.measurements.lock().unwrap().prepare_ms = ms(prepare_start.elapsed());
    let admission_start = Instant::now();
    let lease = quota::begin(claims, "local-benchmark")
        .await?
        .ok_or_else(|| anyhow!("benchmark needs a quota lease"))?;
    trial.measurements.lock().unwrap().admission_ms = ms(admission_start.elapsed());
    let graph_start = Instant::now();
    run.execute(state).await;
    let graph_ms = ms(graph_start.elapsed());
    let validation = bench.workload.validate(&run, trial.action.person_id).await;
    {
        let log = run.run.lock().await;
        let mut measured = trial.measurements.lock().unwrap();
        measured.graph_ms = graph_ms;
        measured.rows = bench.workload.rows_per_person;
        measured.verified = validation.is_ok();
        measured.node_traces = log.traces.iter().map(|trace| json!({"node_id":trace.node_id,"start_ms":ms(trace.start.duration_since(trial.wall_origin).unwrap_or_default()),"duration_ms":ms(trace.end.duration_since(trace.start).unwrap_or_default())})).collect();
    }
    trial.span("graph", graph_start);
    let runtime_ms = lease.elapsed_ms();
    trial
        .delay("durability_delay", bench.options.durability_ms)
        .await;
    lease
        .finish(
            runtime_ms,
            if validation.is_ok() {
                "completed"
            } else {
                "failed"
            },
        )
        .await?;
    validation?;
    if trial
        .measurements
        .lock()
        .unwrap()
        .history_patch_ms
        .is_none()
    {
        return Err(anyhow!("graph emitted no rental-history patch"));
    }
    Ok(())
}

fn bench_rows(trial: &Trial) -> usize {
    trial.measurements.lock().unwrap().rows
}
