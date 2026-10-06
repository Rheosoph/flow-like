//! `ManagementCommand::Models`. Reads need Use models or Manage models (the owner always
//! passes), writes need Manage models, and `Ensure` needs Deploy on its project because it is
//! a step of that project's deploy. A deployer also reads `Jobs`, limited to the downloads
//! its own deploys asked for, so a deploy can follow them before it applies. `Stats` breaks
//! usage down only by the consumers a reader may see. Writes are journaled under the
//! request's operation ID like every other mutating command, so a retry answers the first
//! result.

use super::*;
#[cfg(feature = "runtime")]
use crate::models::host::ModelHost;

/// Room an encrypted reply needs besides its result.
#[cfg(feature = "runtime")]
const REPLY_ENVELOPE_BYTES: usize = 512;
/// How long a write that drains requests in flight holds the management connection.
#[cfg(feature = "runtime")]
const DRAIN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Writes and the hardware probe wait on the host, so they run on the async path.
fn is_async(request: &ModelsRequest) -> bool {
    request.is_write() || matches!(request, ModelsRequest::Probe {})
}

fn may_read(authority: &Authority) -> bool {
    authority.permits(ManagementCapability::ModelUse, None, None)
        || authority.permits(ManagementCapability::ModelManage, None, None)
}

#[cfg(feature = "runtime")]
fn require_read(authority: &Authority) -> Result<()> {
    refuse_unless(
        may_read(authority),
        RejectionCode::Unauthorized,
        "Reading the models of this device needs Use models or Manage models",
    )
}

/// Which jobs a read lists: all, or those the filter accepts by their operation IDs.
type JobFilter<'a> = Option<&'a dyn Fn(&[String]) -> Result<bool>>;

/// Whose usage a `Stats` read breaks down.
type ConsumerFilter<'a> = &'a dyn Fn(&ModelConsumer) -> Result<bool>;

/// What a reader sees of the jobs and of the usage per consumer.
#[derive(Clone, Copy)]
struct Visible<'a> {
    jobs: JobFilter<'a>,
    consumers: ConsumerFilter<'a>,
}

/// What a read may see: everything, or (`Jobs` of a deployer) only its deploys' downloads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Everything,
    DeployedJobs,
}

fn reach_of(authority: &Authority, request: &ModelsRequest) -> Result<Reach> {
    if may_read(authority) {
        return Ok(Reach::Everything);
    }
    let deploys = authority
        .grant
        .as_ref()
        .is_some_and(|grant| grant.capabilities.contains(&ManagementCapability::Deploy));
    refuse_unless(
        deploys && matches!(request, ModelsRequest::Jobs { .. }),
        RejectionCode::Unauthorized,
        "Reading the models of this device needs Use models or Manage models",
    )?;
    Ok(Reach::DeployedJobs)
}

/// The projects whose journaled Ensure asked for a job.
fn ensured_projects(store: &StateStore, operation_ids: &[String]) -> Result<Vec<String>> {
    let mut statement = store.connection.prepare(
        "SELECT project_id FROM management_operations WHERE operation_id=?1 AND project_id IS NOT NULL",
    )?;
    let mut projects = Vec::new();
    for id in operation_ids {
        let project = statement
            .query_row([id], |row| row.get::<_, String>(0))
            .optional()?;
        projects.extend(project);
    }
    Ok(projects)
}

/// Whether a journaled Ensure of a project `authority` may deploy asked for a job.
pub(super) fn deploy_asked(
    store: &StateStore,
    authority: &Authority,
    operation_ids: &[String],
) -> Result<bool> {
    let projects = ensured_projects(store, operation_ids)?;
    Ok(projects
        .iter()
        .any(|project| authority.permits(ManagementCapability::Deploy, Some(project), None)))
}

/// The owner sees everyone's usage and Manage models every person's. Every reader sees their
/// own, and that of each placement whose status they may read; a removed placement's usage
/// stays the owner's to see.
fn shows_usage_of(
    store: &StateStore,
    authority: &Authority,
    consumer: &ModelConsumer,
) -> Result<bool> {
    if authority.grant.is_none() || *consumer == super::tunnel::consumer_of(authority) {
        return Ok(true);
    }
    let ModelConsumer::Placement { placement_id } = consumer else {
        return Ok(authority.permits(ManagementCapability::ModelManage, None, None));
    };
    let project = store.get_placement(placement_id)?.and_then(|record| {
        let project = record.config.get("project_id")?.as_str()?;
        Some(project.to_owned())
    });
    Ok(project.is_some_and(|project| {
        authority.permits(
            ManagementCapability::Status,
            Some(&project),
            Some(placement_id),
        )
    }))
}

/// The project a write is journaled under; an Ensure is authorized like its deploy.
#[cfg(feature = "runtime")]
fn authorize_write(authority: &Authority, request: &ModelsRequest) -> Result<Option<String>> {
    if let ModelsRequest::Ensure { project_id, .. } = request {
        authority.require(ManagementCapability::Deploy, Some(project_id), None)?;
        return Ok(Some(project_id.clone()));
    }
    authority.require(ManagementCapability::ModelManage, None, None)?;
    Ok(None)
}

fn models_request(request: &ManagementRequest) -> Result<&ModelsRequest> {
    match &request.command {
        ManagementCommand::Models { request } => Ok(request),
        _ => anyhow::bail!("Operation {} is not a models command", request.operation_id),
    }
}

/// A completed answer, refused when it exceeds `limit` bytes.
fn answer(operation_id: &str, result: Value, limit: usize) -> Result<ManagementResponse> {
    let response = ManagementResponse {
        operation_id: operation_id.to_owned(),
        state: "completed".into(),
        result,
    };
    let bytes = serde_json::to_vec(&response)?.len();
    refuse_unless(
        bytes <= limit,
        RejectionCode::Limit,
        format!("The models answer of {bytes} bytes exceeds the {limit} bytes one reply carries"),
    )?;
    Ok(response)
}

/// Every read but the probe: on the management connection with `limit` one encrypted
/// message, or as the tunnel's bulk read of `Stats` with 1 MiB.
pub(super) fn read(
    store: &StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    manifest: &OnboardingManifest,
    now: i64,
    limit: usize,
) -> Result<ManagementResponse> {
    fenced_read(
        store,
        authority,
        request,
        (manifest, now),
        limit,
        |models, visible| read_result(models, now, visible),
    )
}

/// The answer `result` gives the reader: a deployer's `Jobs` lists only its deploys' jobs,
/// and `Stats` breaks usage down only by the consumers the reader may see.
fn reached(
    store: &StateStore,
    authority: &Authority,
    models: &ModelsRequest,
    result: impl FnOnce(&ModelsRequest, Visible) -> Result<Value>,
) -> Result<Value> {
    let deployed = |operation_ids: &[String]| deploy_asked(store, authority, operation_ids);
    let jobs: JobFilter = match reach_of(authority, models)? {
        Reach::Everything => None,
        Reach::DeployedJobs => Some(&deployed),
    };
    let consumers = |consumer: &ModelConsumer| shows_usage_of(store, authority, consumer);
    let visible = Visible {
        jobs,
        consumers: &consumers,
    };
    result(models, visible)
}

/// Checks the request and the reader, asks `result` for the answer, and fences it against
/// a grant that changed meanwhile.
fn fenced_read(
    store: &StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    (manifest, now): (&OnboardingManifest, i64),
    limit: usize,
    result: impl FnOnce(&ModelsRequest, Visible) -> Result<Value>,
) -> Result<ManagementResponse> {
    let models = models_request(request)?;
    models.validate().reject_as(RejectionCode::Invalid)?;
    refuse_unless(
        !is_async(models),
        RejectionCode::Invalid,
        "Model writes and the hardware probe run on the management connection",
    )?;
    let result = reached(store, authority, models, result)?;
    authorized_read(
        store,
        authority.read_guard(manifest, request, now, None, None),
        || {
            reach_of(authority, models)?;
            answer(&request.operation_id, result, limit)
        },
    )
}

/// A models command of the management connection. Reads run on a blocking thread; the
/// probe and the writes wait on the host.
pub(super) async fn execute_async(
    service: &Arc<ManagementService>,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    validate_request(request, &service.device.manifest().device_id, now)?;
    let models = models_request(request)?;
    models.validate().reject_as(RejectionCode::Invalid)?;
    if !is_async(models) {
        return read_blocking(service, authority, request, now).await;
    }
    #[cfg(feature = "runtime")]
    {
        let host = running_host()?;
        run_with(&host, service, authority, request, now).await
    }
    #[cfg(not(feature = "runtime"))]
    {
        Err(without_host())
    }
}

async fn read_blocking(
    service: &Arc<ManagementService>,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    let (service, authority) = (Arc::clone(service), authority.clone());
    let request = request.clone();
    tokio::task::spawn_blocking(move || {
        let store = StateStore::open(&service.state_dir.join("management.sqlite"))?;
        let manifest = service.device.manifest();
        read(
            &store,
            &authority,
            &request,
            manifest,
            now,
            noise::MAX_PLAINTEXT,
        )
    })
    .await?
}

#[cfg(not(feature = "runtime"))]
fn read_result(_: &ModelsRequest, _: i64, _: Visible) -> Result<Value> {
    Err(without_host())
}

#[cfg(not(feature = "runtime"))]
fn without_host() -> anyhow::Error {
    refusal(
        RejectionCode::Unsupported,
        "This device agent was built without the model host",
    )
}

/// A host that failed to start stays down until the agent restarts, so only a host that is
/// still starting is worth a retry.
#[cfg(feature = "runtime")]
fn running_host() -> Result<std::sync::Arc<ModelHost>> {
    ModelHost::current().ok_or_else(|| {
        if crate::diagnostics::model_host_failed() {
            refusal(
                RejectionCode::Unsupported,
                "The model host of this device did not start; models stay unavailable until the agent restarts",
            )
        } else {
            refusal(
                RejectionCode::Busy,
                "The model host of this device is not running yet; it starts with the agent",
            )
        }
    })
}

#[cfg(feature = "runtime")]
fn read_result(request: &ModelsRequest, now: i64, visible: Visible) -> Result<Value> {
    let host = running_host()?;
    read_from(&host, request, now, visible)
}

#[cfg(feature = "runtime")]
fn to_json<T: serde::Serialize>(value: Result<T>) -> Result<Value> {
    Ok(serde_json::to_value(value?)?)
}

#[cfg(feature = "runtime")]
fn read_from(
    host: &ModelHost,
    request: &ModelsRequest,
    now: i64,
    visible: Visible,
) -> Result<Value> {
    match request {
        ModelsRequest::Overview {} => overview(host, now),
        ModelsRequest::Models { after, limit } => model_page(host, after.as_deref(), *limit),
        ModelsRequest::Jobs { after, limit } => {
            job_page(host, after.as_deref(), *limit, visible.jobs)
        }
        ModelsRequest::Recommendations { after, limit } => {
            recommendation_page(host, after.as_deref(), *limit)
        }
        ModelsRequest::Stats {
            model_id,
            from,
            to,
            step,
        } => to_json(host.stats().query_shown(
            model_id.as_deref(),
            *from,
            *to,
            *step,
            visible.consumers,
        )),
        _ => anyhow::bail!("Models request {request:?} is not a read"),
    }
}

#[cfg(feature = "runtime")]
fn overview(host: &ModelHost, now: i64) -> Result<Value> {
    use crate::models::recommend;
    let recommendations = recommend::current(host).unwrap_or_else(|error| {
        tracing::warn!("Compute the model recommendations: {error:#}");
        Vec::new()
    });
    let mut overview = ModelsOverview {
        observed_at: now,
        system: recommend::live_facts(host)?,
        runtimes: host.runtimes().infos()?,
        runtime_manifest_url: host.runtimes().manifest_url().map(str::to_owned),
        summary: host.summary()?,
        recommendations: recommendations
            .into_iter()
            .take(MODEL_OVERVIEW_MAX_RECOMMENDATIONS)
            .collect(),
        models: Vec::new(),
        next: None,
    };
    fit_models(&mut overview, host.supervisor().models())?;
    Ok(serde_json::to_value(overview)?)
}

/// The first models by id that fit one reply next to the rest of the overview; `next`
/// names the last one when more follow.
#[cfg(feature = "runtime")]
fn fit_models(overview: &mut ModelsOverview, models: Vec<HostedModel>) -> Result<()> {
    let used = serde_json::to_vec(overview)?.len() + REPLY_ENVELOPE_BYTES;
    let mut budget = noise::MAX_PLAINTEXT.saturating_sub(used);
    let total = models.len();
    for model in models.into_iter().take(usize::from(MODELS_PAGE_MAX)) {
        let bytes = serde_json::to_vec(&model)?.len() + 1;
        if bytes > budget {
            break;
        }
        budget -= bytes;
        overview.models.push(model);
    }
    refuse_unless(
        total == 0 || !overview.models.is_empty(),
        RejectionCode::Limit,
        "One hosted model does not fit the overview; read the models page by page",
    )?;
    if overview.models.len() < total {
        overview.next = overview.models.last().map(|model| model.id.clone());
    }
    Ok(())
}

#[cfg(feature = "runtime")]
fn model_page(host: &ModelHost, after: Option<&str>, limit: u16) -> Result<Value> {
    let rows = host
        .supervisor()
        .models()
        .into_iter()
        .filter(|model| after.is_none_or(|after| model.id.as_str() > after))
        .take(usize::from(limit) + 1)
        .map(serde_json::to_value)
        .collect::<serde_json::Result<Vec<_>>>()?;
    let (models, more) = page(rows, usize::from(limit), "hosted models")?;
    let next = next_cursor(&models, more, "id");
    Ok(json!({"models":models,"next":next}))
}

/// Jobs by ID, so a cursor stays valid while other jobs end; `shown` limits a deployer's.
#[cfg(feature = "runtime")]
fn job_page(host: &ModelHost, after: Option<&str>, limit: u16, shown: JobFilter) -> Result<Value> {
    let mut jobs = Vec::new();
    for job in host.acquisition().jobs() {
        if shown.map_or(Ok(true), |shown| shown(&job.operation_ids))? {
            jobs.push(job);
        }
    }
    jobs.sort_by(|left, right| left.job_id.cmp(&right.job_id));
    let rows = jobs
        .iter()
        .filter(|job| after.is_none_or(|after| job.job_id.as_str() > after))
        .filter_map(listed_job)
        .take(usize::from(limit) + 1)
        .map(serde_json::to_value)
        .collect::<serde_json::Result<Vec<_>>>()?;
    let (jobs, more) = page(rows, usize::from(limit), "model jobs")?;
    let next = next_cursor(&jobs, more, "job_id");
    Ok(json!({"jobs":jobs,"next":next}))
}

/// The job as `Jobs` lists it, or `None` when the protocol refuses it: a client parses a
/// page as a whole, so one such row would fail every read of it.
#[cfg(feature = "runtime")]
fn listed_job(job: &crate::models::acquire::JobSnapshot) -> Option<ModelJob> {
    let listed = job.wire();
    if let Err(error) = listed.validate() {
        tracing::warn!(
            "Model job {} is left out of the jobs page: {error}",
            listed.job_id
        );
        return None;
    }
    Some(listed)
}

/// Recommendations are computed per read, so the cursor counts the ones read before.
#[cfg(feature = "runtime")]
fn recommendation_page(host: &ModelHost, after: Option<&str>, limit: u16) -> Result<Value> {
    let offset = match after {
        Some(after) => after.parse::<usize>().map_err(|_| {
            refusal(
                RejectionCode::Invalid,
                format!("Recommendation cursor {after} is not a count of recommendations"),
            )
        })?,
        None => 0,
    };
    let rows = crate::models::recommend::current(host)?
        .into_iter()
        .skip(offset)
        .take(usize::from(limit) + 1)
        .map(serde_json::to_value)
        .collect::<serde_json::Result<Vec<_>>>()?;
    let (recommendations, more) = page(rows, usize::from(limit), "model recommendations")?;
    let next = more.then(|| (offset + recommendations.len()).to_string());
    Ok(json!({"recommendations":recommendations,"next":next}))
}

/// Retries of one operation wait for its accepted write to finish.
#[cfg(feature = "runtime")]
fn operation_lock(
    service: &ManagementService,
    authority: &Authority,
    request: &ManagementRequest,
) -> Arc<tokio::sync::Mutex<()>> {
    type Key = (PathBuf, String, String);
    type Locks = std::collections::HashMap<Key, std::sync::Weak<tokio::sync::Mutex<()>>>;
    static LOCKS: std::sync::LazyLock<std::sync::Mutex<Locks>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Locks::new()));
    let mut locks = LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, held| held.strong_count() > 0);
    let key = (
        service.state_dir.clone(),
        authority.principal.clone(),
        request.operation_id.clone(),
    );
    let entry = locks.entry(key).or_default();
    let lock = entry
        .upgrade()
        .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
    *entry = Arc::downgrade(&lock);
    lock
}

#[cfg(feature = "runtime")]
async fn run_with(
    host: &Arc<ModelHost>,
    service: &ManagementService,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    let models = models_request(request)?;
    if !models.is_write() {
        return probe(host, service, authority, request, now).await;
    }
    let project = authorize_write(authority, models)?;
    let guard = operation_lock(service, authority, request)
        .lock_owned()
        .await;
    let journal = match Journal::claim(service, authority, request, project.as_deref(), now)? {
        Claim::Replay(response) => return Ok(response),
        Claim::Run(journal) => journal,
    };
    // A disconnected caller does not cancel an accepted write or its journal completion.
    let (host, state_dir, request) = (Arc::clone(host), service.state_dir.clone(), request.clone());
    tokio::spawn(async move {
        let _guard = guard;
        let models = models_request(&request)?;
        let result = if journal.resumed {
            resumed_result(&host, models)?
        } else {
            None
        };
        let result = match result {
            Some(result) => Ok(result),
            None => apply(&host, &state_dir, &request, models).await,
        };
        match result {
            Ok(result) => journal.complete(result),
            Err(error) => {
                journal.abandon();
                Err(error)
            }
        }
    })
    .await?
}

#[cfg(feature = "runtime")]
fn resumed_result(host: &ModelHost, request: &ModelsRequest) -> Result<Option<Value>> {
    match request {
        ModelsRequest::Configure {
            model_id,
            expected_revision,
            settings,
            residency,
        } => {
            let current = host.supervisor().model(model_id);
            match current.filter(|current| {
                Some(current.revision) == expected_revision.checked_add(1)
                    && current.settings == *settings
                    && current.residency == *residency
            }) {
                Some(current) => Ok(Some(serde_json::to_value(current)?)),
                None => Ok(None),
            }
        }
        ModelsRequest::Remove { model_id, .. } if host.supervisor().model(model_id).is_none() => {
            Ok(Some(serde_json::to_value(ModelRemoved {
                model_id: model_id.clone(),
            })?))
        }
        _ => Ok(None),
    }
}

#[cfg(feature = "runtime")]
async fn probe(
    host: &ModelHost,
    service: &ManagementService,
    authority: &Authority,
    request: &ManagementRequest,
    now: i64,
) -> Result<ManagementResponse> {
    require_read(authority)?;
    let started = std::time::Instant::now();
    let facts = serde_json::to_value(host.probe().await?)?;
    let store = StateStore::open(&service.state_dir.join("management.sqlite"))?;
    let now = now.saturating_add(started.elapsed().as_secs() as i64);
    let guard = authority.read_guard(service.device.manifest(), request, now, None, None);
    authorized_read(&store, guard, || {
        require_read(authority)?;
        answer(&request.operation_id, facts, noise::MAX_PLAINTEXT)
    })
}

#[cfg(feature = "runtime")]
async fn apply(
    host: &ModelHost,
    state_dir: &Path,
    request: &ManagementRequest,
    models: &ModelsRequest,
) -> Result<Value> {
    match models {
        ModelsRequest::Install {
            model_id,
            model,
            settings,
            residency,
        } => install(host, model_id, model, settings, *residency),
        ModelsRequest::Configure {
            model_id,
            expected_revision,
            settings,
            residency,
        } => configure(host, model_id, *expected_revision, settings, *residency).await,
        ModelsRequest::Load { model_id } => load(host, model_id),
        ModelsRequest::Unload { model_id } => unload(host, model_id).await,
        ModelsRequest::Remove {
            model_id,
            expected_revision,
        } => remove(host, model_id, *expected_revision).await,
        ModelsRequest::Ensure { project_id, pins } => {
            ensure(host, state_dir, project_id, pins, &request.operation_id).await
        }
        ModelsRequest::InstallRuntime {
            runtime,
            backend,
            manifest_jws,
        } => to_json(
            host.runtimes()
                .install_with_manifest(*runtime, *backend, manifest_jws.as_deref())
                .await,
        ),
        ModelsRequest::RemoveRuntime { runtime, backend } => {
            remove_runtime(host, *runtime, *backend)
        }
        ModelsRequest::CancelJob { job_id } => cancel_job(host, job_id).await,
        _ => anyhow::bail!("Models request {models:?} is not a write"),
    }
}

/// The hosted model a write names; a model removed meanwhile is a stale read.
#[cfg(feature = "runtime")]
fn hosted(host: &ModelHost, model_id: &str) -> Result<HostedModel> {
    host.supervisor().model(model_id).ok_or_else(|| {
        refusal(
            RejectionCode::RevisionConflict,
            format!("Model {model_id} is not hosted on this device"),
        )
    })
}

#[cfg(feature = "runtime")]
fn require_revision(host: &ModelHost, model_id: &str, expected: u64) -> Result<()> {
    let revision = hosted(host, model_id)?.revision;
    refuse_unless(
        revision == expected,
        RejectionCode::RevisionConflict,
        format!("Model {model_id} is at revision {revision}, not {expected}"),
    )
}

#[cfg(feature = "runtime")]
fn install(
    host: &ModelHost,
    model_id: &str,
    spec: &ModelSpec,
    settings: &ModelSettings,
    residency: Residency,
) -> Result<Value> {
    let existing = host.store().with_db(|db| db.hosted_model(model_id))?;
    refuse_unless(
        existing.is_none_or(|record| record.spec == *spec),
        RejectionCode::RevisionConflict,
        format!("Install model {model_id}: this id already hosts another model"),
    )?;
    let (model, assets) = host.supervisor().install(
        model_id,
        spec.clone(),
        settings.clone(),
        residency,
        crate::models::db::ModelOrigin::User,
    )?;
    to_json(Ok(ModelInstalled { model, assets }))
}

/// Waits at most `DRAIN_WAIT` for a write that lets requests in flight finish first. A write
/// still draining then goes on by itself, its failure only logged, and `meanwhile` answers.
#[cfg(feature = "runtime")]
async fn drained(
    model_id: &str,
    mut write: tokio::task::JoinHandle<Result<Value>>,
    meanwhile: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    if let Ok(written) = tokio::time::timeout(DRAIN_WAIT, &mut write).await {
        return written?;
    }
    let model_id = model_id.to_owned();
    tokio::spawn(async move {
        if let Ok(Err(error)) = write.await {
            tracing::warn!(model = %model_id, "A models write that outlasted its answer failed: {error:#}");
        }
    });
    meanwhile()
}

#[cfg(feature = "runtime")]
async fn configure(
    host: &ModelHost,
    model_id: &str,
    expected_revision: u64,
    settings: &ModelSettings,
    residency: Residency,
) -> Result<Value> {
    require_revision(host, model_id, expected_revision)?;
    to_json(
        host.supervisor()
            .configure(model_id, expected_revision, settings.clone(), residency)
            .await,
    )
}

#[cfg(feature = "runtime")]
fn load(host: &ModelHost, model_id: &str) -> Result<Value> {
    refuse_unless(
        hosted(host, model_id)?.residency != Residency::PinnedOff,
        RejectionCode::Invalid,
        format!("Model {model_id} is pinned off; give it another residency to load it"),
    )?;
    to_json(host.supervisor().load(model_id))
}

#[cfg(feature = "runtime")]
async fn unload(host: &ModelHost, model_id: &str) -> Result<Value> {
    hosted(host, model_id)?;
    let (supervisor, id) = (host.supervisor().clone(), model_id.to_owned());
    let write = tokio::spawn(async move { to_json(supervisor.unload(&id).await) });
    drained(model_id, write, || to_json(hosted(host, model_id))).await
}

#[cfg(feature = "runtime")]
async fn remove(host: &ModelHost, model_id: &str, expected_revision: u64) -> Result<Value> {
    require_revision(host, model_id, expected_revision)?;
    refuse_unless(
        !host.supervisor().placement_uses(model_id)?,
        RejectionCode::RevisionConflict,
        format!(
            "Model {model_id} is used by a placement; remove that placement's model dependency first"
        ),
    )?;
    host.supervisor()
        .remove(model_id, expected_revision)
        .await?;
    to_json(Ok(ModelRemoved {
        model_id: model_id.to_owned(),
    }))
}

#[cfg(feature = "runtime")]
async fn ensure(
    host: &ModelHost,
    state_dir: &Path,
    project_id: &str,
    pins: &[ProjectBitPin],
    operation_id: &str,
) -> Result<Value> {
    to_json(
        crate::project_artifacts::ensure_project_models(
            state_dir,
            host.acquisition(),
            project_id,
            pins,
            Some(operation_id),
        )
        .await,
    )
}

#[cfg(feature = "runtime")]
fn remove_runtime(host: &ModelHost, runtime: ModelRuntime, backend: ModelBackend) -> Result<Value> {
    if let Some(model) = host.supervisor().runtime_in_use(runtime, backend) {
        return Err(refusal(
            RejectionCode::RevisionConflict,
            format!(
                "Remove runtime {runtime:?}/{backend:?}: loaded model {model} runs from it; unload it first"
            ),
        ));
    }
    to_json(host.remove_runtime(runtime, backend))
}

/// Answers the job as `Jobs` lists it, with the state the cancellation left.
#[cfg(feature = "runtime")]
async fn cancel_job(host: &ModelHost, job_id: &str) -> Result<Value> {
    let acquisition = host.acquisition();
    let open = |acquisition: &crate::models::acquire::AcquisitionManager| {
        acquisition
            .jobs()
            .into_iter()
            .find(|job| job.job_id == job_id)
    };
    let job = open(acquisition).ok_or_else(|| {
        refusal(
            RejectionCode::RevisionConflict,
            format!("Model job {job_id} is not open on this device"),
        )
    })?;
    let state = acquisition.cancel(&job.asset.digest).await?;
    let mut answer = open(acquisition).unwrap_or(job).wire();
    answer.state = state;
    to_json(Ok(answer))
}

#[cfg(feature = "runtime")]
enum Claim {
    Replay(ManagementResponse),
    Run(Journal),
}

/// The journal row of one write: claimed before the host acts, completed with the answer,
/// and dropped again when the write fails, so a refused write leaves no trace.
#[cfg(feature = "runtime")]
struct Journal {
    database: PathBuf,
    operation_id: String,
    digest: String,
    principal: String,
    resumed: bool,
}

#[cfg(feature = "runtime")]
impl Journal {
    fn claim(
        service: &ManagementService,
        authority: &Authority,
        request: &ManagementRequest,
        project: Option<&str>,
        now: i64,
    ) -> Result<Claim> {
        let mut journal = Self {
            database: service.state_dir.join("management.sqlite"),
            operation_id: request.operation_id.clone(),
            digest: compact_digest(&serde_json::to_string(request)?),
            principal: authority.principal.clone(),
            resumed: false,
        };
        let store = StateStore::open(&journal.database)?;
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let manifest = service.device.manifest();
        match journal.reserve(&store, authority, manifest, project, now) {
            Ok(previous) => {
                store.connection.execute_batch("COMMIT")?;
                Ok(previous.map_or(Claim::Run(journal), Claim::Replay))
            }
            Err(error) => {
                let _ = store.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// The first answer of a completed write with this ID, else a pending row for it. A
    /// pending row of an interrupted attempt runs again; every write is idempotent.
    fn reserve(
        &mut self,
        store: &StateStore,
        authority: &Authority,
        manifest: &OnboardingManifest,
        project: Option<&str>,
        now: i64,
    ) -> Result<Option<ManagementResponse>> {
        authority.require_current(store, manifest, now)?;
        let previous = previous_operation(
            &store.connection,
            &self.operation_id,
            &self.digest,
            authority,
        )?;
        if let Some(previous) = previous {
            self.resumed = previous.state == "pending";
            return Ok((previous.state != "pending").then_some(previous));
        }
        reserve_journal_entry(&store.connection, authority, now)?;
        let pending = ManagementResponse {
            operation_id: self.operation_id.clone(),
            state: "pending".into(),
            result: json!({"command":"models"}),
        };
        store.connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,accepted_at,result_json) VALUES(?1,?2,?3,?4,?5,?6)",params![self.operation_id,self.digest,self.principal,project,now,serde_json::to_string(&pending)?])?;
        Ok(None)
    }

    fn complete(self, result: Value) -> Result<ManagementResponse> {
        let response = answer(&self.operation_id, result, noise::MAX_PLAINTEXT);
        let Ok(response) = response else {
            self.abandon();
            return response;
        };
        let store = StateStore::open(&self.database)?;
        store.connection.execute("UPDATE management_operations SET result_json=?2 WHERE operation_id=?1 AND request_digest=?3 AND principal=?4",params![self.operation_id,serde_json::to_string(&response)?,self.digest,self.principal])?;
        Ok(response)
    }

    fn abandon(&self) {
        let removed = StateStore::open(&self.database).and_then(|store| {
            Ok(store.connection.execute("DELETE FROM management_operations WHERE operation_id=?1 AND request_digest=?2 AND principal=?3 AND json_extract(result_json,'$.state')='pending'",params![self.operation_id,self.digest,self.principal])?)
        });
        if let Err(error) = removed {
            tracing::warn!(
                operation_id = %self.operation_id,
                "The pending journal row of a failed models write stays until it expires: {error:#}"
            );
        }
    }
}

#[cfg(all(test, feature = "runtime"))]
mod tests {
    use super::*;
    use crate::models::{
        engines::fake::{FakeBehaviour, FakeLauncher},
        fetch::{AddressPolicy, Fetcher},
        host::{HostConfig, HostParts},
    };
    use serde::de::DeserializeOwned;
    use std::{io::Write, sync::Arc};

    struct Fixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        service: Arc<ManagementService>,
        host: Arc<ModelHost>,
        signer: SigningKey,
        now: i64,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("a state directory");
        let root = crate::supervisor::prepare_state_dir(directory.path()).expect("agent state");
        let signer = SigningKey::generate();
        let device = DeviceSession::test_management_session(
            "http://127.0.0.1:1/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            SigningKey::generate().public_key(),
        )
        .test_with_invitation_key(signer.public_key());
        let service = ManagementService::new(root.clone(), Arc::new(device), "boot".into());
        let parts = HostParts {
            fetcher: Fetcher::new(AddressPolicy::global_only()).expect("a fetcher"),
            runtime_source: None,
            launcher: Arc::new(FakeLauncher {
                behaviour: FakeBehaviour::default(),
            }),
        };
        let host = ModelHost::start(&root, HostConfig::default(), parts)
            .await
            .expect("a model host");
        Fixture {
            _directory: directory,
            root,
            service,
            host,
            signer,
            now: unix_time().expect("a clock"),
        }
    }

    fn code(result: Result<ManagementResponse>) -> String {
        match result {
            Ok(response) => panic!("expected a refusal, got {response:?}"),
            Err(error) => rejection_code(&error).as_str().to_owned(),
        }
    }

    fn completed(id: &str) -> (String, String) {
        (id.to_owned(), "completed".to_owned())
    }

    impl Fixture {
        fn owner(&self) -> Authority {
            Authority {
                principal: "owner:owner".into(),
                key: self.service.device.manifest().controller_key.clone(),
                grant: None,
            }
        }

        fn grant(
            &self,
            id: &str,
            scope: ManagementScope,
            capabilities: Vec<ManagementCapability>,
        ) -> Authority {
            let key = SigningKey::generate().public_key();
            Authority {
                principal: format!("{id}:{id}:key"),
                key: key.clone(),
                grant: Some(ManagementGrant {
                    grant_id: id.into(),
                    user_id: id.into(),
                    controller_key: key,
                    scope,
                    capabilities,
                    expires_at: self.now + 3_600,
                    group_id: None,
                    group_version: None,
                }),
            }
        }

        /// Device grants that may use models, manage them, and only read the status.
        fn device_grants(&self) -> [Authority; 3] {
            let grants = [
                ("user", ManagementCapability::ModelUse),
                ("manager", ManagementCapability::ModelManage),
                ("status", ManagementCapability::Status),
            ]
            .map(|(id, capability)| self.grant(id, ManagementScope::Device, vec![capability]));
            self.accept(&grants.each_ref());
            grants
        }

        fn accept(&self, grantees: &[&Authority]) {
            let policy = ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: 1,
                previous_policy_digest: None,
                grants: grantees
                    .iter()
                    .filter_map(|authority| authority.grant.clone())
                    .collect(),
                issued_at: self.now - 10,
                expires_at: self.now + 3_600,
            };
            let signed = sign_management_policy(&policy, &self.signer).expect("a signed policy");
            self.store()
                .accept_management_policy(&signed, &self.signer.public_key(), "device", self.now)
                .expect("an accepted policy");
        }

        fn store(&self) -> StateStore {
            StateStore::open(&self.root.join("management.sqlite")).expect("the management store")
        }

        fn request(&self, id: &str, request: ModelsRequest) -> ManagementRequest {
            ManagementRequest {
                operation_id: id.into(),
                device_id: "device".into(),
                issued_at: self.now,
                expires_at: self.now + 60,
                command: ManagementCommand::Models { request },
            }
        }

        async fn send(
            &self,
            authority: &Authority,
            id: &str,
            models: ModelsRequest,
        ) -> Result<ManagementResponse> {
            self.send_within(authority, id, models, noise::MAX_PLAINTEXT)
                .await
        }

        async fn send_within(
            &self,
            authority: &Authority,
            id: &str,
            models: ModelsRequest,
            limit: usize,
        ) -> Result<ManagementResponse> {
            let request = self.request(id, models);
            if is_async(models_request(&request)?) {
                return run_with(&self.host, &self.service, authority, &request, self.now).await;
            }
            let read = |models: &ModelsRequest, visible: Visible| {
                read_from(&self.host, models, self.now, visible)
            };
            let manifest = self.service.device.manifest();
            let store = self.store();
            fenced_read(
                &store,
                authority,
                &request,
                (manifest, self.now),
                limit,
                read,
            )
        }

        /// The completed answer of a request, parsed.
        async fn ok<T: DeserializeOwned>(
            &self,
            authority: &Authority,
            id: &str,
            models: ModelsRequest,
        ) -> T {
            let response = self
                .send(authority, id, models)
                .await
                .unwrap_or_else(|error| panic!("{id} was refused: {error:#}"));
            assert_eq!(response.state, "completed");
            serde_json::from_value(response.result).expect("a well-formed answer")
        }

        /// The rejection code of a refused request.
        async fn refused(&self, authority: &Authority, id: &str, models: ModelsRequest) -> String {
            code(self.send(authority, id, models).await)
        }

        fn journaled(&self) -> Vec<(String, String)> {
            let store = self.store();
            let mut rows = store
                .connection
                .prepare(
                    "SELECT operation_id,json_extract(result_json,'$.state') FROM management_operations ORDER BY operation_id",
                )
                .expect("a journal query");
            rows.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .and_then(Iterator::collect)
                .expect("the journal")
        }

        /// A chat model whose one asset is in the store already.
        fn spec(&self, name: &str) -> ModelSpec {
            let bytes = name.as_bytes();
            let asset = ModelAssetDescriptor {
                digest: ModelAssetDigest {
                    algorithm: DigestAlgorithm::Blake3,
                    hex: blake3::hash(bytes).to_hex().to_string(),
                },
                size: bytes.len() as u64,
                file_name: format!("{name}.gguf"),
                sources: vec![],
            };
            let store = self.host.store();
            let mut staged = store.open_partial(&asset.digest).expect("a staged file");
            staged.write_all(bytes).expect("staged bytes");
            store.publish(&asset).expect("a stored asset");
            chat_spec(name, asset)
        }

        async fn install(&self, name: &str) -> ModelInstalled {
            let spec = self.spec(name);
            self.ok(&self.owner(), name, install(name, spec)).await
        }

        /// The open job of a missing asset, asked for by `operation_id` when there is one.
        fn open_job(&self, hex: &str, operation_id: Option<&str>) -> String {
            let asset = ModelAssetDescriptor {
                digest: ModelAssetDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    hex: hex.repeat(64),
                },
                size: 4_096,
                file_name: format!("{hex}.gguf"),
                sources: vec![],
            };
            let ensured = self.host.acquisition().ensure(&asset, operation_id);
            let ensured = ensured.expect("an ensured asset");
            ensured.job_id.expect("an open job")
        }
    }

    fn chat_spec(name: &str, asset: ModelAssetDescriptor) -> ModelSpec {
        ModelSpec {
            display_name: name.into(),
            kind: ModelKind::Chat,
            engine: ModelEngine::Llamacpp,
            assets: vec![asset],
            projector: None,
            pooling: None,
        }
    }

    /// Pinned off, so no engine starts.
    fn install(model_id: &str, model: ModelSpec) -> ModelsRequest {
        ModelsRequest::Install {
            model_id: model_id.into(),
            model,
            settings: ModelSettings::default(),
            residency: Residency::PinnedOff,
        }
    }

    fn configure(expected_revision: u64) -> ModelsRequest {
        ModelsRequest::Configure {
            model_id: "qwen".into(),
            expected_revision,
            settings: ModelSettings {
                parallel: Some(2),
                ..ModelSettings::default()
            },
            residency: Residency::default(),
        }
    }

    fn remove(expected_revision: u64) -> ModelsRequest {
        ModelsRequest::Remove {
            model_id: "qwen".into(),
            expected_revision,
        }
    }

    fn qwen() -> String {
        "qwen".into()
    }

    #[tokio::test]
    async fn reads_need_use_models_or_manage_models() {
        let fixture = fixture().await;
        let [user, manager, status] = fixture.device_grants();
        for reader in [&fixture.owner(), &user, &manager] {
            let overview: ModelsOverview = fixture
                .ok(reader, "overview", ModelsRequest::Overview {})
                .await;
            overview.validate().expect("a valid overview");
            let facts: SystemFacts = fixture.ok(reader, "probe", ModelsRequest::Probe {}).await;
            facts.validate().expect("valid hardware facts");
        }
        for read in [ModelsRequest::Overview {}, ModelsRequest::Probe {}] {
            assert_eq!(fixture.refused(&status, "read", read).await, "unauthorized");
        }
        assert!(fixture.journaled().is_empty(), "reads are never journaled");
    }

    #[tokio::test]
    async fn writes_need_manage_models_and_only_completed_ones_are_journaled() {
        let fixture = fixture().await;
        let [user, manager, _] = fixture.device_grants();
        let qwen = install("qwen", fixture.spec("qwen"));
        let refused = fixture
            .refused(&user, "install-as-user", qwen.clone())
            .await;
        assert_eq!(refused, "unauthorized");
        let installed: ModelInstalled = fixture.ok(&manager, "install-as-manager", qwen).await;
        assert_eq!(installed.model.id, "qwen");
        assert_eq!((installed.assets.total, installed.assets.present), (1, 1));
        assert_eq!(fixture.journaled(), [completed("install-as-manager")]);
    }

    #[tokio::test]
    async fn a_retried_write_answers_its_first_result() {
        let fixture = fixture().await;
        let owner = fixture.owner();
        let spec = fixture.spec("qwen");
        let first: ModelInstalled = fixture
            .ok(&owner, "qwen", install("qwen", spec.clone()))
            .await;
        let again: ModelInstalled = fixture.ok(&owner, "qwen", install("qwen", spec)).await;
        assert_eq!(first, again);
        let load = ModelsRequest::Load { model_id: qwen() };
        assert_eq!(fixture.refused(&owner, "qwen", load).await, "invalid");
        assert_eq!(fixture.journaled(), [completed("qwen")]);
    }

    #[tokio::test]
    async fn stale_revisions_and_pinned_models_are_refused_without_a_journal_row() {
        let fixture = fixture().await;
        let owner = fixture.owner();
        fixture.install("qwen").await;
        let load = ModelsRequest::Load { model_id: qwen() };
        assert_eq!(fixture.refused(&owner, "load", load).await, "invalid");
        let stale = fixture.refused(&owner, "stale", configure(7)).await;
        assert_eq!(stale, "revision_conflict");
        let configured: HostedModel = fixture.ok(&owner, "configure", configure(1)).await;
        assert_eq!(configured.revision, 2);
        assert_eq!(configured.settings.parallel, Some(2));
        let unload = ModelsRequest::Unload { model_id: qwen() };
        let unloaded: HostedModel = fixture.ok(&owner, "unload", unload).await;
        assert_eq!(unloaded.state, HostedModelState::Stopped);
        let stale = fixture.refused(&owner, "remove-stale", remove(1)).await;
        assert_eq!(stale, "revision_conflict");
        let removed: ModelRemoved = fixture.ok(&owner, "remove", remove(2)).await;
        assert_eq!(removed.model_id, "qwen");
        assert!(fixture.host.supervisor().models().is_empty());
        let journal = ["configure", "qwen", "remove", "unload"].map(completed);
        assert_eq!(fixture.journaled(), journal);
    }

    #[tokio::test]
    async fn ensure_is_authorized_like_the_deploy_of_its_project() {
        let fixture = fixture().await;
        let project = ManagementScope::Project {
            project_id: "project".into(),
        };
        let deployer = fixture.grant("deployer", project, vec![ManagementCapability::Deploy]);
        let manager = fixture.grant(
            "manager",
            ManagementScope::Device,
            vec![ManagementCapability::ModelManage],
        );
        fixture.accept(&[&deployer, &manager]);
        let ensure = |project: &str| ModelsRequest::Ensure {
            project_id: project.into(),
            pins: vec![ProjectBitPin {
                bit_id: "bit".into(),
                metadata_sha256: "a".repeat(64),
            }],
        };
        let other = fixture.refused(&deployer, "other", ensure("other")).await;
        assert_eq!(other, "unauthorized");
        let manager_ensure = fixture
            .refused(&manager, "manager", ensure("project"))
            .await;
        assert_eq!(manager_ensure, "unauthorized");
        let overview = ModelsRequest::Overview {};
        assert_eq!(
            fixture.refused(&deployer, "overview", overview).await,
            "unauthorized"
        );
        let uncommitted = fixture
            .refused(&deployer, "ensure", ensure("project"))
            .await;
        assert_ne!(
            uncommitted, "unauthorized",
            "the deployer passed authorization"
        );
        assert!(fixture.journaled().is_empty());
    }

    #[tokio::test]
    async fn model_pages_continue_after_their_cursor() {
        let fixture = fixture().await;
        let owner = fixture.owner();
        for name in ["c-model", "a-model", "b-model"] {
            fixture.install(name).await;
        }
        let page = |after: Option<&str>| ModelsRequest::Models {
            after: after.map(str::to_owned),
            limit: 2,
        };
        let first: HostedModelPage = fixture.ok(&owner, "page-1", page(None)).await;
        let ids: Vec<_> = first.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["a-model", "b-model"]);
        assert_eq!(first.next.as_deref(), Some("b-model"));
        let second: HostedModelPage = fixture.ok(&owner, "page-2", page(Some("b-model"))).await;
        assert_eq!(second.models.len(), 1);
        assert_eq!(second.next, None);
    }

    #[tokio::test]
    async fn jobs_list_and_cancel_the_acquisition_of_a_missing_asset() {
        let fixture = fixture().await;
        let owner = fixture.owner();
        let missing = ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: "b".repeat(64),
            },
            size: 1_000,
            file_name: "missing.gguf".into(),
            sources: vec![],
        };
        let spec = chat_spec("missing", missing);
        let installed: ModelInstalled = fixture
            .ok(&owner, "missing", install("missing", spec))
            .await;
        let job_id = installed.assets.pending[0].job_id.clone();
        let job_id = job_id.expect("a job for the missing asset");
        let jobs = ModelsRequest::Jobs {
            after: None,
            limit: 8,
        };
        let jobs: ModelJobPage = fixture.ok(&owner, "jobs", jobs).await;
        assert_eq!(jobs.jobs.len(), 1);
        assert_eq!(jobs.jobs[0].job_id, job_id);
        let cancel = ModelsRequest::CancelJob { job_id };
        let cancelled: ModelJob = fixture.ok(&owner, "cancel", cancel).await;
        assert!(matches!(
            cancelled.state,
            ModelAssetState::Failed {
                reason: ModelAssetFailure::Cancelled,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn deployers_read_only_the_jobs_their_deploys_asked_for() {
        let fixture = fixture().await;
        let project = |project_id: &str| ManagementScope::Project {
            project_id: project_id.into(),
        };
        let deploy = || vec![ManagementCapability::Deploy];
        let deployer = fixture.grant("deployer", project("project"), deploy());
        let stranger = fixture.grant("stranger", project("other"), deploy());
        let status = vec![ManagementCapability::Status];
        let status = fixture.grant("status", ManagementScope::Device, status);
        fixture.accept(&[&deployer, &stranger, &status]);
        let asked = fixture.open_job("c", Some("ensure-op"));
        fixture.open_job("d", None);
        fixture.store().connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,accepted_at,result_json) VALUES('ensure-op','digest','deployer','project',?1,'{}')", [fixture.now]).expect("a journaled ensure");
        let jobs = || ModelsRequest::Jobs {
            after: None,
            limit: 8,
        };
        let all: ModelJobPage = fixture.ok(&fixture.owner(), "owner", jobs()).await;
        assert_eq!(all.jobs.len(), 2);
        let deployed: ModelJobPage = fixture.ok(&deployer, "deployer", jobs()).await;
        let ids: Vec<_> = deployed.jobs.into_iter().map(|job| job.job_id).collect();
        assert_eq!((ids, deployed.next), (vec![asked], None));
        let other: ModelJobPage = fixture.ok(&stranger, "stranger", jobs()).await;
        assert!(other.jobs.is_empty(), "another project's deployer saw jobs");
        let refused = fixture.refused(&status, "status", jobs()).await;
        assert_eq!(refused, "unauthorized");
    }

    #[test]
    fn a_job_the_protocol_refuses_is_left_out_of_the_jobs_page() {
        let fetching = |source_host: &str| crate::models::acquire::JobSnapshot {
            job_id: uuid::Uuid::new_v4().to_string(),
            asset: ModelAssetDescriptor {
                digest: ModelAssetDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    hex: "a".repeat(64),
                },
                size: 4_096,
                file_name: "a.gguf".into(),
                sources: vec![],
            },
            state: ModelAssetState::Fetching {
                source_index: 0,
                bytes: 1,
            },
            source_host: Some(source_host.into()),
            attempts: 1,
            bytes_per_second: None,
            operation_ids: Vec::new(),
            updated_at: 1,
        };
        assert!(listed_job(&fetching("cdn.flow-like.com")).is_some());
        assert!(listed_job(&fetching("my_cdn.example.com")).is_none());
    }

    #[tokio::test]
    async fn statistics_beyond_one_message_need_the_bulk_read() {
        let fixture = fixture().await;
        let owner = fixture.owner();
        let to = fixture.now - fixture.now.rem_euclid(60);
        let stats = ModelsRequest::Stats {
            model_id: None,
            from: to - MODEL_STATS_MAX_POINTS as i64 * 60,
            to,
            step: StatsStep::Minute,
        };
        assert_eq!(
            fixture.refused(&owner, "stats", stats.clone()).await,
            "limit"
        );
        let bulk = fixture
            .send_within(&owner, "stats", stats, MODELS_BULK_REPLY_MAX_BYTES)
            .await
            .expect("a bulk answer");
        let stats: ModelStats = serde_json::from_value(bulk.result).expect("model stats");
        assert_eq!(stats.series.requests.len(), MODEL_STATS_MAX_POINTS);
        let cursor = ModelsRequest::Recommendations {
            after: Some("first".into()),
            limit: 4,
        };
        assert_eq!(fixture.refused(&owner, "cursor", cursor).await, "invalid");
    }

    #[tokio::test]
    async fn interrupted_revision_writes_resume_their_applied_result() -> Result<()> {
        let fixture = fixture().await;
        fixture.install("qwen").await;
        let owner = fixture.owner();
        for (id, models) in [
            ("configure-resume", configure(1)),
            ("remove-resume", remove(2)),
        ] {
            let request = fixture.request(id, models.clone());
            assert!(matches!(
                Journal::claim(&fixture.service, &owner, &request, None, fixture.now)?,
                Claim::Run(_)
            ));
            let applied = apply(&fixture.host, &fixture.root, &request, &models).await?;
            let replayed = fixture.send(&owner, id, models.clone()).await?;
            assert_eq!(replayed.result, applied);
            assert_eq!(fixture.send(&owner, id, models).await?.result, applied);
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_disconnected_remove_is_journaled_only_after_its_requests_drain() -> Result<()> {
        use crate::models::db::RuntimeRecord;
        let fixture = fixture().await;
        let runtime = fixture.root.join("runtimes/llamacpp/test-cpu");
        std::fs::create_dir_all(&runtime)?;
        std::fs::write(runtime.join("llama-server"), b"#!/bin/sh\n")?;
        fixture.host.store().with_db(|db| {
            db.put_runtime(&RuntimeRecord {
                runtime: ModelRuntime::Llamacpp,
                backend: ModelBackend::Cpu,
                build: "test".into(),
                entrypoint: "llama-server".into(),
                size: 1,
                needs_fallback: false,
                installed_at: 0,
            })
        })?;
        fixture.install("qwen").await;
        fixture
            .host
            .supervisor()
            .configure("qwen", 1, ModelSettings::default(), Residency::default())
            .await?;
        let lease = fixture.host.supervisor().acquire("qwen").await?;
        let (host, service, owner, request, now) = (
            Arc::clone(&fixture.host),
            Arc::clone(&fixture.service),
            fixture.owner(),
            fixture.request("draining-remove", remove(2)),
            fixture.now,
        );
        let caller =
            tokio::spawn(async move { run_with(&host, &service, &owner, &request, now).await });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !fixture
            .journaled()
            .iter()
            .any(|(id, _)| id == "draining-remove")
        {
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        caller.abort();
        tokio::time::sleep(DRAIN_WAIT + std::time::Duration::from_millis(100)).await;
        assert!(fixture.host.supervisor().model("qwen").is_some());
        assert!(
            fixture
                .journaled()
                .contains(&("draining-remove".into(), "pending".into()))
        );
        drop(lease);
        let response = fixture
            .send(&fixture.owner(), "draining-remove", remove(2))
            .await?;
        assert_eq!(response.state, "completed");
        assert!(fixture.host.supervisor().model("qwen").is_none());
        fixture.host.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_model_used_by_a_placement_cannot_be_removed() -> Result<()> {
        use crate::models::db::{AssetOwner, OwnerKind};
        let fixture = fixture().await;
        let installed = fixture.install("qwen").await;
        let owner = AssetOwner::new(OwnerKind::Placement, "placement")?;
        fixture
            .host
            .store()
            .add_ref(&installed.model.assets[0], &owner)?;
        assert_eq!(
            fixture
                .refused(&fixture.owner(), "remove-used", remove(1))
                .await,
            "revision_conflict"
        );
        assert!(fixture.host.supervisor().model("qwen").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn statistics_hide_other_people_and_unreadable_placements() -> Result<()> {
        use crate::models::db::RequestRecord;
        let fixture = fixture().await;
        let [user, manager, _] = fixture.device_grants();
        let consumer = |id: &str| ModelConsumer::Grant {
            grant_id: id.into(),
        };
        let from = fixture.now - fixture.now.rem_euclid(60);
        let identities = [
            consumer("user"),
            consumer("other"),
            ModelConsumer::Placement {
                placement_id: "hidden".into(),
            },
        ];
        let rows: Vec<_> = identities
            .into_iter()
            .map(|consumer| RequestRecord {
                at: from,
                model_id: "qwen".into(),
                consumer,
                status: 200,
                prompt_tokens: 1,
                completion_tokens: 1,
                cached_tokens: 0,
                ttft_ms: None,
                duration_ms: 1,
                decode_ms: 1,
                queue_ms: 0,
                request_bytes: 1,
                response_bytes: 1,
            })
            .collect();
        fixture.host.stats().write(&rows)?;
        let request = || ModelsRequest::Stats {
            model_id: None,
            from,
            to: from + 60,
            step: StatsStep::Minute,
        };
        for (reader, expected) in [(&user, 1), (&manager, 2), (&fixture.owner(), 3)] {
            let result: ModelStats = fixture.ok(reader, "consumer-stats", request()).await;
            assert_eq!(result.consumers.len(), expected);
            assert_eq!(result.series.requests, [3]);
            if expected == 1 {
                assert_eq!(result.consumers[0].consumer, consumer("user"));
            }
        }
        Ok(())
    }
}
