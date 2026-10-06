//! The device's model host: store, acquisition, runtime packs, supervisor, statistics and
//! the loopback gateway, started once per agent.

use super::{
    acquire::{AcquisitionConfig, AcquisitionManager},
    engines::{EngineLauncher, ProcessLauncher},
    fetch::{AddressPolicy, Fetcher},
    gateway::{Gateway, GatewayConfig},
    runtime::{RuntimeInstaller, RuntimeSource},
    stats::Stats,
    store::{ModelStore, ModelStoreConfig},
    supervisor::{Supervisor, SupervisorConfig},
    system,
};
use crate::enrollment::unix_time;
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    ModelBackend, ModelRuntime, ModelsSummary, ReleaseTarget, RuntimeInfo, RuntimeInstalled,
    SystemFacts,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

static CURRENT: RwLock<Option<Weak<ModelHost>>> = RwLock::new(None);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default)]
pub struct HostConfig {
    pub store: ModelStoreConfig,
    pub acquisition: AcquisitionConfig,
    pub supervisor: SupervisorConfig,
    pub gateway: GatewayConfig,
}

impl HostConfig {
    pub fn from_state(state_dir: &Path) -> Result<Self> {
        let mut config = Self::default();
        config.store.max_bytes = crate::isolation::models_max_bytes(state_dir)?;
        Ok(config)
    }
}

/// What a host is built from; production derives every part from the state directory.
pub struct HostParts {
    pub fetcher: Fetcher,
    pub runtime_source: Option<RuntimeSource>,
    pub launcher: Arc<dyn EngineLauncher>,
}

impl HostParts {
    /// Global-only fetches, the release trust's runtime manifest, and engines sandboxed on
    /// Linux whenever the host can, which a host requiring isolation must.
    pub fn production(state_dir: &Path) -> Result<Self> {
        let target = ReleaseTarget::current()?;
        let runtime_source = RuntimeSource::from_trust(state_dir, target)
            .inspect_err(|error| tracing::info!("Runtime packs are unavailable: {error:#}"))
            .ok();
        let capabilities = crate::isolation::capabilities(state_dir);
        let sandbox = cfg!(target_os = "linux")
            && capabilities.bubblewrap.is_some()
            && capabilities.landlock_abi.is_some()
            && capabilities.cgroup_root.is_some();
        ensure!(
            sandbox || !capabilities.require_isolation,
            "This device requires isolation, but engines cannot be sandboxed: {}",
            capabilities.reason.unwrap_or_default()
        );
        Ok(Self {
            fetcher: Fetcher::new(AddressPolicy::global_only())?,
            runtime_source,
            launcher: Arc::new(ProcessLauncher {
                sandbox,
                state_dir: Some(state_dir.to_owned()),
            }),
        })
    }
}

pub struct ModelHost {
    state_dir: PathBuf,
    acquisition: AcquisitionManager,
    runtimes: Arc<RuntimeInstaller>,
    supervisor: Supervisor,
    stats: Arc<Stats>,
    gateway: Gateway,
    cancel: CancellationToken,
}

impl ModelHost {
    /// Starts every part inside the current Tokio runtime and makes this host the one
    /// placements reach through [`ModelHost::current`].
    pub async fn start(
        state_dir: &Path,
        config: HostConfig,
        parts: HostParts,
    ) -> Result<Arc<Self>> {
        let cancel = CancellationToken::new();
        let images = parts.fetcher.clone();
        let (acquisition, runtimes) =
            storage(state_dir, &config, parts.fetcher, parts.runtime_source)?;
        let facts = probe(&runtimes, acquisition.store().root()).await?;
        let supervisor = Supervisor::start(
            state_dir,
            acquisition.clone(),
            Arc::clone(&runtimes),
            parts.launcher,
            facts,
            config.supervisor,
            cancel.child_token(),
        )?;
        let stats = Arc::new(Stats::open(
            &acquisition.store().root().join("models.sqlite"),
        )?);
        let recorder = stats.start(cancel.child_token());
        let gateway = Gateway::start(
            supervisor.clone(),
            recorder,
            config.gateway,
            images,
            cancel.child_token(),
        );
        let host = Arc::new(Self {
            state_dir: state_dir.to_owned(),
            acquisition,
            runtimes,
            supervisor,
            stats,
            gateway: gateway.await?,
            cancel,
        });
        *CURRENT
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::downgrade(&host));
        Ok(host)
    }

    /// The host this agent started, while it runs.
    pub fn current() -> Option<Arc<Self>> {
        CURRENT
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()?
            .upgrade()
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn acquisition(&self) -> &AcquisitionManager {
        &self.acquisition
    }

    pub fn store(&self) -> &Arc<ModelStore> {
        self.acquisition.store()
    }

    pub fn runtimes(&self) -> &Arc<RuntimeInstaller> {
        &self.runtimes
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.supervisor
    }

    pub fn stats(&self) -> &Arc<Stats> {
        &self.stats
    }

    pub fn gateway(&self) -> &Gateway {
        &self.gateway
    }

    /// Runs the hardware probe again; installed runtimes report GPUs with their memory.
    pub async fn probe(&self) -> Result<SystemFacts> {
        let facts = probe(&self.runtimes, self.store().root()).await?;
        self.supervisor.set_facts(facts.clone());
        Ok(facts)
    }

    pub async fn install_runtime(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<RuntimeInstalled> {
        self.runtimes.install(runtime, backend).await
    }

    /// Refused while a loaded model runs from the runtime.
    pub fn remove_runtime(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<RuntimeInfo> {
        if let Some(model) = self.supervisor.runtime_in_use(runtime, backend) {
            anyhow::bail!("Remove the runtime: loaded model {model} runs from it; unload it first");
        }
        self.runtimes.remove(runtime, backend)
    }

    /// Headline counters of the Models tab and the fleet snapshot.
    pub fn summary(&self) -> Result<ModelsSummary> {
        let models = self.supervisor.models();
        let count = |predicate: fn(&flow_like_device_protocol::HostedModelState) -> bool| {
            u16::try_from(
                models
                    .iter()
                    .filter(|model| predicate(&model.state))
                    .count(),
            )
            .unwrap_or(u16::MAX)
        };
        let day = self.stats.day_totals(unix_time()?)?;
        let store_bytes = self.store().with_db(|db| db.stored_bytes())?;
        Ok(ModelsSummary {
            models: u16::try_from(models.len()).unwrap_or(u16::MAX),
            loaded: count(|state| {
                matches!(
                    state,
                    flow_like_device_protocol::HostedModelState::Loaded { .. }
                )
            }),
            failed: count(|state| {
                matches!(
                    state,
                    flow_like_device_protocol::HostedModelState::Failed { .. }
                )
            }),
            requests_24h: day.requests,
            tokens_24h: day.tokens,
            errors_24h: day.errors,
            store_bytes,
            store_budget_bytes: store_bytes.saturating_add(self.store().available_bytes()?),
        })
    }

    /// Stops engines, the gateway and the statistics writer; journaled downloads resume
    /// on the next start.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let _ = tokio::time::timeout(SHUTDOWN_WAIT, self.acquisition.shutdown()).await;
    }
}

impl Drop for ModelHost {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests;

/// The store, its acquisition engine and the runtime installer on top of it.
fn storage(
    state_dir: &Path,
    config: &HostConfig,
    fetcher: Fetcher,
    runtime_source: Option<RuntimeSource>,
) -> Result<(AcquisitionManager, Arc<RuntimeInstaller>)> {
    let store = Arc::new(ModelStore::open(state_dir, config.store.clone())?);
    let acquisition = AcquisitionManager::start(store, fetcher, config.acquisition.clone())?;
    let runtimes = RuntimeInstaller::open(
        state_dir,
        acquisition.clone(),
        runtime_source,
        ReleaseTarget::current()?,
    )?;
    Ok((acquisition, runtimes))
}

async fn probe(runtimes: &RuntimeInstaller, volume: &Path) -> Result<SystemFacts> {
    let installed = runtimes
        .installed_of(ModelRuntime::Llamacpp)
        .context("Read the installed runtimes")?;
    let mut listed = None;
    for runtime in installed {
        match system::runtime_gpus(runtime.command()).await {
            Ok(found) => {
                listed = Some(found);
                break;
            }
            Err(error) => tracing::warn!(
                "List GPUs with runtime {}: {error:#}",
                runtime.dir.display()
            ),
        }
    }
    let gpus = match listed {
        Some(found) if !found.is_empty() => found,
        Some(_) => system::unseen_by_runtime(system::os_gpus().await),
        None => system::os_gpus().await,
    };
    system::probe(volume, gpus)
}
