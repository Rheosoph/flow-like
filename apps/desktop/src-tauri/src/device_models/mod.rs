//! Models hosted on Flow-Like devices, for runs in the desktop app. A run that names a device Bit
//! gets an OpenAI-compatible endpoint on loopback, and behind it one tunnel per device reaches the
//! model gateway of that device. Controller keys stay in this process, bound to the account and
//! grant that unlocked them. A locked device is unlocked from a prompt or in the device area.
//! Kept keys stay until the user locks the device, signs out or quits. Keys the device area holds
//! go when it lets go of them, unless a run used them within 30 minutes; other keys lock 30
//! minutes after their last use.

mod link;
mod native;
mod prompt;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use flow_like::models::device::{
    DeviceModelConnector, DeviceModelTarget, Interaction, ModelEndpoint, ModelUnavailable,
    ModelUnavailableReason,
};
use flow_like_device_client::DeviceSession;
use tauri::{AppHandle, Manager, Webview};
use tokio::time::Instant;

use link::{DeviceLink, Failure, Holder, PromptHost, UnlockError, UnlockPrompt};
pub(crate) use link::{HeldKeys, VaultUnlock};
use native::NativeLink;
use prompt::{Prompts, Question};

const PROMPT_EVENT: &str = "device-model-unlock-requested";
const CLOSED_EVENT: &str = "device-model-unlock-closed";
const HELD_EVENT: &str = "device-models-held";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    /// An unanswered prompt counts as a decline after this long.
    pub(crate) prompt: Duration,
    /// Keys the user did not keep lock after this long without a use.
    pub(crate) idle_lock: Duration,
    pub(crate) idle_check: Duration,
    /// A device that could not be reached is answered as offline for this long instead of
    /// being connected to again; a connect to an offline device takes about 36 s.
    pub(crate) retry_offline: Duration,
}

/// Who asked for the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unlocker {
    /// A run's unlock prompt.
    Prompt,
    /// The device area of the main window, which decides when it lets go.
    Area,
}

const TIMING: Timing = Timing {
    prompt: Duration::from_secs(120),
    idle_lock: Duration::from_secs(30 * 60),
    idle_check: Duration::from_secs(60),
    retry_offline: Duration::from_secs(30),
};

static CONNECTOR: LazyLock<Arc<DeviceModels<NativeLink, WindowPrompts>>> =
    LazyLock::new(|| DeviceModels::new(NativeLink::new(), WindowPrompts::default(), TIMING));

/// The connector runs reach device models through.
pub(crate) fn connector() -> Arc<dyn DeviceModelConnector> {
    CONNECTOR.clone()
}

/// Until the app is attached, every prompt is an immediate decline.
pub(crate) fn attach(app: &AppHandle) {
    CONNECTOR.prompts.host().attach(app.clone());
}

/// Zeroizes every controller key this process unlocked; called when the app exits.
pub(crate) fn lock_all() {
    native::lock_all();
}

/// Locks every device: kept keys, keys a prompt unlocked and keys the device area holds.
pub(crate) async fn lock_every_device() {
    CONNECTOR.lock_all().await;
    native::lock_all();
}

/// Keys of accounts no window is signed in to any more lock; called after a sign-out or an
/// account switch.
pub(crate) async fn drop_signed_out() {
    CONNECTOR.drop_signed_out().await;
}

/// The native session of a device whose keys this process holds. Model calls, local ports and
/// transfers of the device area share its one tunnel.
pub(crate) async fn session(device_id: &str) -> Result<DeviceSession, String> {
    flow_like_device_protocol::validate_management_id(device_id)
        .map_err(|error| format!("Device id {device_id:?} is invalid: {error}"))?;
    match CONNECTOR.session(device_id).await {
        Some(Ok(session)) => Ok(session),
        Some(Err(failure)) => Err(format!(
            "Could not connect to device {device_id}: {}",
            failure.detail
        )),
        None => Err(format!(
            "The desktop app holds no unlocked keys for device {device_id}; lock it and unlock it again in Devices"
        )),
    }
}

/// Unlocks a device for the device area of the main window. The keys stay while the area holds
/// them; true when they are also kept for model access.
pub(crate) async fn unlock_for_area(unlock: VaultUnlock) -> Result<bool, String> {
    CONNECTOR
        .unlock(unlock, Unlocker::Area)
        .await
        .map_err(|error| error.to_string())
}

/// The user locked a device: its keys go, kept or not, with its tunnel and model gateway.
pub(crate) async fn lock(device_id: &str) {
    CONNECTOR.lock(device_id).await;
}

/// The device area let go of a device on its own (an idle lock, a page that is gone).
pub(crate) async fn release(device_id: &str) {
    CONNECTOR.release(device_id).await;
}

/// The device area's "Keep unlocked for model access" changed.
pub(crate) async fn keep(device_id: &str, keep: bool) {
    CONNECTOR.keep(device_id, keep).await;
}

pub(crate) struct DeviceModels<L: DeviceLink, H: PromptHost> {
    link: L,
    prompts: Arc<Prompts<H>>,
    timing: Timing,
    devices: Mutex<HashMap<String, Arc<Device<L>>>>,
    generations: AtomicU64,
    /// Moves on every lock of all devices, so an unlock that started earlier installs nothing.
    lock_epoch: AtomicU64,
    held: Mutex<HashMap<String, HeldKeys>>,
}

struct Device<L: DeviceLink> {
    id: String,
    name: Mutex<Option<String>>,
    /// Moves on every lock of this device.
    locks: AtomicU64,
    slot: tokio::sync::Mutex<Option<Held<L>>>,
    /// One gateway open at a time. Opens run outside `slot`, so a lock, a local port or a
    /// transfer never waits for a slow connect.
    opening: tokio::sync::Mutex<()>,
}

fn sorted(held: &HashMap<String, HeldKeys>) -> Vec<HeldKeys> {
    let mut held: Vec<HeldKeys> = held.values().cloned().collect();
    held.sort_by(|a, b| a.device_id.cmp(&b.device_id));
    held
}

/// Keys an unlock opened, before they are held.
struct Opening<K> {
    keys: K,
    holder: Holder,
    kept: bool,
    area: bool,
}

struct Held<L: DeviceLink> {
    /// Shared with an open or a connect in flight; letting go closes them for both.
    keys: Arc<L::Keys>,
    holder: Holder,
    /// Kept for model access: only a lock, a sign-out or quitting drops them.
    kept: bool,
    /// The device area holds them; they never lock on their own while it does.
    area: bool,
    generation: u64,
    /// The unlock, or the last run, local port or transfer that used the keys.
    last_used: Instant,
    /// The last run that connected to the model gateway; the unlock does not count.
    used: Option<Instant>,
    gateway: Option<Arc<L::Gateway>>,
    /// When opening the gateway last found the device offline, and why.
    offline: Option<(Instant, Failure)>,
}

/// What the held keys of a device offer a model call now.
enum Ready<L: DeviceLink> {
    Served(Result<Arc<L::Gateway>, Failure>),
    /// No gateway yet: open one with these keys, while they are of this generation.
    Open(Arc<L::Keys>, u64),
}

impl<L: DeviceLink> Held<L> {
    /// A request runs through the gateway now, or a use was within `idle`; `since_unlock`
    /// counts the unlock and local ports as uses.
    fn busy(&self, link: &L, idle: Duration, since_unlock: bool) -> bool {
        let activity = self
            .gateway
            .as_ref()
            .map(|gateway| link.activity(gateway))
            .unwrap_or_default();
        if activity.in_flight > 0 {
            return true;
        }
        let used = if since_unlock {
            Some(self.last_used)
        } else {
            self.used
        };
        [used, activity.last_request]
            .into_iter()
            .flatten()
            .max()
            .is_some_and(|at| at.elapsed() < idle)
    }

    fn summary(&self, device: &Device<L>) -> HeldKeys {
        HeldKeys {
            device_id: device.id.clone(),
            device_name: device.name(),
            api_origin: self.holder.api_origin.clone(),
            account: self.holder.account.clone(),
            kept: self.kept,
            area: self.area,
        }
    }
}

impl<L: DeviceLink> Device<L> {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: Mutex::new(None),
            locks: AtomicU64::new(0),
            slot: tokio::sync::Mutex::new(None),
            opening: tokio::sync::Mutex::new(()),
        }
    }

    fn name(&self) -> Option<String> {
        self.name
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_name(&self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            *self.name.lock().unwrap_or_else(PoisonError::into_inner) = Some(name.to_owned());
        }
    }

    /// "Model 'Qwen3 8B' runs on device 'GPU box', which is offline (cause)."
    fn unavailable(&self, target: &DeviceModelTarget, failure: Failure) -> ModelUnavailable {
        let sentence = ModelUnavailable::for_target(failure.reason, target, self.name().as_deref());
        if failure.detail.is_empty() {
            return sentence;
        }
        ModelUnavailable::new(
            failure.reason,
            format!(
                "{} ({}).",
                sentence.message.trim_end_matches('.'),
                failure.detail.trim_end_matches('.')
            ),
        )
    }
}

impl<L: DeviceLink, H: PromptHost> DeviceModels<L, H> {
    pub(crate) fn new(link: L, host: H, timing: Timing) -> Arc<Self> {
        Arc::new(Self {
            link,
            prompts: Prompts::new(host, timing.prompt),
            timing,
            devices: Mutex::default(),
            generations: AtomicU64::new(1),
            lock_epoch: AtomicU64::new(0),
            held: Mutex::default(),
        })
    }

    fn device(&self, device_id: &str) -> Arc<Device<L>> {
        self.devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(device_id.to_owned())
            .or_insert_with(|| Arc::new(Device::new(device_id)))
            .clone()
    }

    fn known(&self, device_id: &str) -> Option<Arc<Device<L>>> {
        self.devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(device_id)
            .cloned()
    }

    fn all(&self) -> Vec<Arc<Device<L>>> {
        self.devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// The devices whose keys are held, by device id.
    pub(crate) fn held(&self) -> Vec<HeldKeys> {
        sorted(&self.held.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Records what `device` holds now and tells the host when that changed. The host hears
    /// every change in order, as the registry stays locked until it was told.
    fn publish(&self, device: &Device<L>, held: Option<&Held<L>>) {
        let summary = held.map(|held| held.summary(device));
        let mut map = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if map.get(&device.id) == summary.as_ref() {
            return;
        }
        match summary {
            Some(summary) => map.insert(device.id.clone(), summary),
            None => map.remove(&device.id),
        };
        self.prompts.host().held(&sorted(&map));
    }

    /// Holds the keys of an opened vault and answers every prompt waiting for the device. Keys of
    /// the same account and grant stay, so a serving tunnel survives, and the new unlock can only
    /// hold them longer; keys of another account or grant are replaced. True when the keys are
    /// kept for model access.
    pub(crate) async fn unlock(
        self: &Arc<Self>,
        vault: VaultUnlock,
        by: Unlocker,
    ) -> Result<bool, UnlockError> {
        let device_id = vault.device_id.clone();
        let device = self.device(&device_id);
        let since = self.lock_count(&device);
        let (holder, kept, area) = (
            Holder::of(&vault),
            vault.keep_unlocked,
            by == Unlocker::Area,
        );
        let unlocked = self.link.unlock(vault).await?;
        device.set_name(&unlocked.device_name);
        let opening = Opening {
            keys: unlocked.keys,
            holder,
            kept,
            area,
        };
        let (kept, watch) = {
            let mut slot = device.slot.lock().await;
            self.admit(&device, since, &opening.holder)?;
            let watch = self.hold(&mut slot, opening);
            self.publish(&device, slot.as_ref());
            (slot.as_ref().is_some_and(|held| held.kept), watch)
        };
        if let Some(generation) = watch {
            self.watch_idle(&device, generation);
        }
        self.prompts.unlocked(&device_id);
        Ok(kept)
    }

    /// Both lock counters of `device`; an unlock installs keys only while they stay the same.
    fn lock_count(&self, device: &Device<L>) -> (u64, u64) {
        (
            self.lock_epoch.load(Ordering::Acquire),
            device.locks.load(Ordering::Acquire),
        )
    }

    /// Refuses keys of a device locked since `since`, or of an account no window is signed in to.
    fn admit(
        &self,
        device: &Device<L>,
        since: (u64, u64),
        holder: &Holder,
    ) -> Result<(), UnlockError> {
        if self.lock_count(device) != since {
            return Err(UnlockError::new(
                "locked",
                format!(
                    "device {} was locked while it unlocked; unlock it again",
                    device.id
                ),
            ));
        }
        if !self.link.signed_in(holder) {
            return Err(UnlockError::new(
                "signed_out",
                format!(
                    "no window is signed in to {} as account {}",
                    holder.api_origin, holder.account
                ),
            ));
        }
        Ok(())
    }

    /// Keys of the account and grant already held stay and are held longer; any others are
    /// replaced. A new unlock tries an offline device again at once. Returns the generation to
    /// watch when nothing holds the new keys.
    fn hold(&self, slot: &mut Option<Held<L>>, opening: Opening<L::Keys>) -> Option<u64> {
        if let Some(held) = slot.as_mut().filter(|held| held.holder == opening.holder) {
            held.kept |= opening.kept;
            held.area |= opening.area;
            held.last_used = Instant::now();
            held.offline = None;
            return None;
        }
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let watch = (!opening.kept && !opening.area).then_some(generation);
        let replaced = slot.replace(Held {
            keys: Arc::new(opening.keys),
            holder: opening.holder,
            kept: opening.kept,
            area: opening.area,
            generation,
            last_used: Instant::now(),
            used: None,
            gateway: None,
            offline: None,
        });
        if let Some(replaced) = replaced {
            self.link.close(&replaced.keys);
        }
        watch
    }

    /// Lets go of the keys `slot` holds; they close now, even while an open still holds them.
    fn clear(&self, slot: &mut Option<Held<L>>) {
        if let Some(held) = slot.take() {
            self.link.close(&held.keys);
        }
    }

    /// The user locked the device: its keys go, kept or not, and its gateway and tunnel close
    /// with them. An unlock of the device still in flight installs nothing.
    pub(crate) async fn lock(&self, device_id: &str) {
        let device = self.device(device_id);
        device.locks.fetch_add(1, Ordering::AcqRel);
        let mut slot = device.slot.lock().await;
        self.clear(&mut slot);
        self.publish(&device, None);
    }

    /// Locks every device, kept or not; unlocks still in flight install nothing.
    pub(crate) async fn lock_all(&self) {
        self.lock_epoch.fetch_add(1, Ordering::AcqRel);
        for device in self.all() {
            let mut slot = device.slot.lock().await;
            self.clear(&mut slot);
            self.publish(&device, None);
        }
    }

    /// The device area let go of the keys on its own: an idle lock or a page that is gone. Kept
    /// keys stay, and so do keys a run used within the idle time or is using now, until they
    /// idle; the rest lock now.
    pub(crate) async fn release(self: &Arc<Self>, device_id: &str) {
        let Some(device) = self.known(device_id) else {
            return;
        };
        let watch = {
            let mut slot = device.slot.lock().await;
            let Some(held) = slot.as_mut().filter(|held| held.area) else {
                return;
            };
            held.area = false;
            let watch = if held.kept {
                None
            } else if held.busy(&self.link, self.timing.idle_lock, false) {
                Some(held.generation)
            } else {
                self.clear(&mut slot);
                None
            };
            self.publish(&device, slot.as_ref());
            watch
        };
        if let Some(generation) = watch {
            self.watch_idle(&device, generation);
        }
    }

    /// The device area kept the keys for model access, or stopped keeping them.
    pub(crate) async fn keep(self: &Arc<Self>, device_id: &str, keep: bool) {
        let Some(device) = self.known(device_id) else {
            return;
        };
        let watch = {
            let mut slot = device.slot.lock().await;
            let Some(held) = slot.as_mut() else {
                return;
            };
            held.kept = keep;
            if !keep {
                held.last_used = Instant::now();
            }
            let watch = (!keep && !held.area).then_some(held.generation);
            self.publish(&device, slot.as_ref());
            watch
        };
        if let Some(generation) = watch {
            self.watch_idle(&device, generation);
        }
    }

    /// Keys of an account no window is signed in to any more lock.
    pub(crate) async fn drop_signed_out(&self) {
        for device in self.all() {
            let mut slot = device.slot.lock().await;
            if slot
                .as_ref()
                .is_some_and(|held| !self.link.signed_in(&held.holder))
            {
                self.clear(&mut slot);
                self.publish(&device, None);
            }
        }
    }

    fn watch_idle(self: &Arc<Self>, device: &Arc<Device<L>>, generation: u64) {
        let models = Arc::downgrade(self);
        let device = Arc::downgrade(device);
        let check = self.timing.idle_check;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(check).await;
                let (Some(models), Some(device)) = (models.upgrade(), device.upgrade()) else {
                    return;
                };
                if !models.lock_if_idle(&device, generation).await {
                    return;
                }
            }
        });
    }

    /// Locks the keys of `generation` once nothing used them for the idle time. False once there
    /// is nothing left to watch: the keys are gone, kept, or held by the device area.
    async fn lock_if_idle(&self, device: &Device<L>, generation: u64) -> bool {
        let mut slot = device.slot.lock().await;
        let Some(held) = slot
            .as_ref()
            .filter(|held| held.generation == generation && !held.kept && !held.area)
        else {
            return false;
        };
        if held.busy(&self.link, self.timing.idle_lock, true) {
            return true;
        }
        self.clear(&mut slot);
        self.publish(device, None);
        false
    }

    /// `None` while the device is locked. A failed model listing fails this call only: the
    /// gateway stays for the requests it serves, and the session replaces a lost tunnel itself.
    async fn serve(
        &self,
        device: &Device<L>,
        target: &DeviceModelTarget,
    ) -> Option<Result<ModelEndpoint, Failure>> {
        let gateway = match self.gateway(device).await? {
            Ok(gateway) => gateway,
            Err(failure) => return Some(Err(failure)),
        };
        let served = self
            .link
            .hosts(&gateway, &target.model)
            .await
            .and_then(|hosted| {
                if hosted {
                    Ok(self.link.endpoint(&gateway, &target.model))
                } else {
                    Err(Failure::new(
                        ModelUnavailableReason::ModelMissing,
                        format!("its model gateway lists no model {}", target.model),
                    ))
                }
            });
        Some(served)
    }

    /// The gateway of the device, opened on first use; `None` while the device is locked. A
    /// device found offline is answered as offline for `retry_offline` without connecting
    /// again. The open runs outside `slot`, and keys locked meanwhile get no gateway.
    async fn gateway(&self, device: &Device<L>) -> Option<Result<Arc<L::Gateway>, Failure>> {
        if let Ready::Served(served) = self.ready(device).await? {
            return Some(served);
        }
        let _opening = device.opening.lock().await;
        let (keys, generation) = match self.ready(device).await? {
            Ready::Served(served) => return Some(served),
            Ready::Open(keys, generation) => (keys, generation),
        };
        let opened = self.link.open(&keys).await;
        let mut slot = device.slot.lock().await;
        let held = slot.as_mut().filter(|held| held.generation == generation)?;
        Some(match opened {
            Ok(gateway) => {
                let gateway = Arc::new(gateway);
                held.gateway = Some(gateway.clone());
                held.offline = None;
                Ok(gateway)
            }
            Err(failure) => {
                if failure.reason == ModelUnavailableReason::DeviceOffline {
                    held.offline = Some((Instant::now(), failure.clone()));
                }
                Err(failure)
            }
        })
    }

    /// What the held keys offer now; each call counts as a use. `None` while locked.
    async fn ready(&self, device: &Device<L>) -> Option<Ready<L>> {
        let mut slot = device.slot.lock().await;
        let held = slot.as_mut()?;
        let now = Instant::now();
        held.last_used = now;
        held.used = Some(now);
        if let Some(gateway) = &held.gateway {
            return Some(Ready::Served(Ok(gateway.clone())));
        }
        if let Some((at, failure)) = &held.offline
            && at.elapsed() < self.timing.retry_offline
        {
            return Some(Ready::Served(Err(failure.clone())));
        }
        Some(Ready::Open(held.keys.clone(), held.generation))
    }

    fn question(
        &self,
        device: &Device<L>,
        target: &DeviceModelTarget,
        run_label: Option<String>,
    ) -> Question {
        Question {
            device_id: device.id.clone(),
            device_name: device.name(),
            model_name: target.display_name.clone(),
            run_label,
        }
    }
}

impl<H: PromptHost> DeviceModels<NativeLink, H> {
    /// `None` while the device is locked here, also when it locked during the connect; a use
    /// counts against the idle lock. The connect runs outside `slot`.
    async fn session(&self, device_id: &str) -> Option<Result<DeviceSession, Failure>> {
        let device = self.device(device_id);
        let (keys, generation) = {
            let mut slot = device.slot.lock().await;
            let held = slot.as_mut()?;
            held.last_used = Instant::now();
            (held.keys.clone(), held.generation)
        };
        let session = self.link.session(&keys).await;
        let current = device
            .slot
            .lock()
            .await
            .as_ref()
            .map(|held| held.generation);
        (current == Some(generation)).then_some(session)
    }
}

#[async_trait]
impl<L: DeviceLink, H: PromptHost> DeviceModelConnector for DeviceModels<L, H> {
    async fn connect(
        &self,
        target: &DeviceModelTarget,
        interaction: Interaction,
    ) -> Result<ModelEndpoint, ModelUnavailable> {
        let device = self.device(&target.device_id);
        let mut asked = false;
        loop {
            if let Some(served) = self.serve(&device, target).await {
                return served.map_err(|failure| device.unavailable(target, failure));
            }
            let run_label = match &interaction {
                Interaction::Allowed { run_label } if !asked => run_label.clone(),
                _ => return Err(device.unavailable(target, Failure::locked())),
            };
            if !self
                .prompts
                .ask(self.question(&device, target, run_label))
                .await
            {
                return Err(device.unavailable(target, Failure::locked()));
            }
            asked = true;
        }
    }
}

/// Prompts in the main window, which mounts the dialog; other windows ignore the events.
#[derive(Default)]
pub(crate) struct WindowPrompts {
    app: OnceLock<AppHandle>,
}

impl WindowPrompts {
    fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }
}

impl PromptHost for WindowPrompts {
    fn can_prompt(&self) -> bool {
        self.app
            .get()
            .and_then(|app| app.get_webview_window("main"))
            .is_some_and(|window| window.is_visible().unwrap_or(false))
    }

    fn show(&self, prompt: &UnlockPrompt) {
        if let Some(app) = self.app.get() {
            crate::utils::emit_to_ui(app, PROMPT_EVENT, prompt.clone());
        }
    }

    fn close(&self, prompt_id: &str) {
        if let Some(app) = self.app.get() {
            crate::utils::emit_to_ui(app, CLOSED_EVENT, prompt_id.to_owned());
        }
    }

    fn run_name(&self, run_label: &str) -> Option<String> {
        let state = self
            .app
            .get()?
            .try_state::<crate::state::TauriFlowLikeState>()?;
        let run = state.0.get_run(run_label).ok()?;
        run.event_name
            .as_deref()
            .or(run.board_name.as_deref())
            .map(str::to_owned)
    }

    fn held(&self, held: &[HeldKeys]) {
        if let Some(app) = self.app.get() {
            crate::utils::emit_to_ui(app, HELD_EVENT, held.to_vec());
            crate::tray::show_device_keys(app, held.len());
        }
    }
}

fn from_main_window(webview: &Webview) -> Result<(), String> {
    if webview.window().label() == "main" {
        Ok(())
    } else {
        Err("invalid: device unlock prompts are answered in the main window".to_owned())
    }
}

/// `ICapabilities.deviceModels`: runs in this app reach models hosted on devices.
#[tauri::command]
pub(crate) fn device_models_available(app: AppHandle) -> bool {
    app.try_state::<crate::state::TauriFlowLikeState>()
        .is_some_and(|state| state.0.device_model_connector.is_some())
}

#[tauri::command]
pub(crate) fn device_models_prompts(webview: Webview) -> Result<Vec<UnlockPrompt>, String> {
    from_main_window(&webview)?;
    Ok(CONNECTOR.prompts.pending())
}

/// The devices whose keys this app holds, also those a run's prompt unlocked; `device-models-held`
/// carries every later change.
#[tauri::command]
pub(crate) fn device_models_held(webview: Webview) -> Result<Vec<HeldKeys>, String> {
    from_main_window(&webview)?;
    Ok(CONNECTOR.held())
}

/// Errors read `<code>: <message>`, `code` one of `invalid`, `wrong_password`,
/// `authority_mismatch`, `device_unavailable`, `hub_unreachable`, `signed_out` or `locked`.
#[tauri::command]
pub(crate) async fn device_models_unlock(
    webview: Webview,
    unlock: VaultUnlock,
) -> Result<(), String> {
    from_main_window(&webview)?;
    CONNECTOR
        .unlock(unlock, Unlocker::Prompt)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn device_models_decline(webview: Webview, prompt_id: String) -> Result<(), String> {
    from_main_window(&webview)?;
    if CONNECTOR.prompts.decline(&prompt_id) {
        Ok(())
    } else {
        Err(format!(
            "ended: the unlock prompt {prompt_id} already ended"
        ))
    }
}
