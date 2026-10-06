use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use flow_like::models::device::DeviceModelKind;
use flow_like_device_client::ProxyActivity;
use serde_json::json;

use super::{link::Unlocked, *};

const DEVICE: &str = "device-1";
const OTHER_DEVICE: &str = "device-2";
const ACCOUNT: &str = "user-1";
const OTHER_ACCOUNT: &str = "user-2";
const PASSWORD: &str = "correct horse battery";
const MODEL: &str = "qwen3-8b";
const FAST: Timing = Timing {
    prompt: Duration::from_millis(400),
    idle_lock: Duration::from_millis(150),
    idle_check: Duration::from_millis(20),
    retry_offline: Duration::from_millis(120),
};

#[derive(Default)]
struct Counters {
    open_attempts: AtomicUsize,
    opened: AtomicUsize,
    keys_closed: AtomicUsize,
    keys_dropped: AtomicUsize,
    gateways_closed: AtomicUsize,
}

struct FakeLink {
    counters: Arc<Counters>,
    open_failure: Mutex<Option<Failure>>,
    open_delay: Mutex<Option<Duration>>,
    hosts_failure: Mutex<Option<Failure>>,
    models: Vec<String>,
    activity: Mutex<ProxyActivity>,
    signed_out: Mutex<HashSet<String>>,
    unlock_delay: Mutex<Option<Duration>>,
}

struct FakeKeys(Arc<Counters>);

impl Drop for FakeKeys {
    fn drop(&mut self) {
        self.0.keys_dropped.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeGateway {
    port: usize,
    counters: Arc<Counters>,
}

impl Drop for FakeGateway {
    fn drop(&mut self) {
        self.counters.gateways_closed.fetch_add(1, Ordering::SeqCst);
    }
}

fn taken(slot: &Mutex<Option<Failure>>) -> Option<Failure> {
    slot.lock().unwrap().take()
}

#[async_trait]
impl DeviceLink for FakeLink {
    type Keys = FakeKeys;
    type Gateway = FakeGateway;

    async fn unlock(&self, vault: VaultUnlock) -> Result<Unlocked<FakeKeys>, UnlockError> {
        let delay = *self.unlock_delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        if vault.password.into_bytes().as_slice() != PASSWORD.as_bytes() {
            return Err(UnlockError::new(
                "wrong_password",
                "the controller vault did not open",
            ));
        }
        Ok(Unlocked {
            keys: FakeKeys(self.counters.clone()),
            device_name: " GPU box ".to_owned(),
        })
    }

    fn close(&self, _keys: &FakeKeys) {
        self.counters.keys_closed.fetch_add(1, Ordering::SeqCst);
    }

    async fn open(&self, _keys: &FakeKeys) -> Result<FakeGateway, Failure> {
        self.counters.open_attempts.fetch_add(1, Ordering::SeqCst);
        let delay = *self.open_delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        if let Some(failure) = taken(&self.open_failure) {
            return Err(failure);
        }
        let port = 41_000 + self.counters.opened.fetch_add(1, Ordering::SeqCst);
        Ok(FakeGateway {
            port,
            counters: self.counters.clone(),
        })
    }

    async fn hosts(&self, _gateway: &FakeGateway, model: &str) -> Result<bool, Failure> {
        match taken(&self.hosts_failure) {
            Some(failure) => Err(failure),
            None => Ok(self.models.iter().any(|hosted| hosted == model)),
        }
    }

    fn endpoint(&self, gateway: &FakeGateway, model: &str) -> ModelEndpoint {
        ModelEndpoint {
            base_url: format!("http://127.0.0.1:{}/v1", gateway.port),
            bearer: "proxy-bearer".to_owned(),
            model: model.to_owned(),
        }
    }

    fn activity(&self, _gateway: &FakeGateway) -> ProxyActivity {
        *self.activity.lock().unwrap()
    }

    fn signed_in(&self, holder: &Holder) -> bool {
        !self.signed_out.lock().unwrap().contains(&holder.account)
    }
}

#[derive(Default)]
struct FakeWindow {
    hidden: AtomicBool,
    shown: Mutex<Vec<UnlockPrompt>>,
    closed: Mutex<Vec<String>>,
    held: Mutex<Vec<Vec<HeldKeys>>>,
}

impl PromptHost for Arc<FakeWindow> {
    fn can_prompt(&self) -> bool {
        !self.hidden.load(Ordering::SeqCst)
    }

    fn show(&self, prompt: &UnlockPrompt) {
        self.shown.lock().unwrap().push(prompt.clone());
    }

    fn close(&self, prompt_id: &str) {
        self.closed.lock().unwrap().push(prompt_id.to_owned());
    }

    fn run_name(&self, run_label: &str) -> Option<String> {
        (run_label == "run-1").then(|| "Daily summary".to_owned())
    }

    fn held(&self, held: &[HeldKeys]) {
        self.held.lock().unwrap().push(held.to_vec());
    }
}

struct Harness {
    connector: Arc<DeviceModels<FakeLink, Arc<FakeWindow>>>,
    window: Arc<FakeWindow>,
    counters: Arc<Counters>,
}

impl Harness {
    fn new() -> Self {
        let counters = Arc::new(Counters::default());
        let window = Arc::new(FakeWindow::default());
        let link = FakeLink {
            counters: counters.clone(),
            open_failure: Mutex::new(None),
            open_delay: Mutex::new(None),
            hosts_failure: Mutex::new(None),
            models: vec![MODEL.to_owned()],
            activity: Mutex::new(ProxyActivity::default()),
            signed_out: Mutex::new(HashSet::new()),
            unlock_delay: Mutex::new(None),
        };
        Self {
            connector: DeviceModels::new(link, window.clone(), FAST),
            window,
            counters,
        }
    }

    fn shown(&self) -> Vec<UnlockPrompt> {
        self.window.shown.lock().unwrap().clone()
    }

    fn closed(&self) -> Vec<String> {
        self.window.closed.lock().unwrap().clone()
    }

    fn dropped(&self) -> usize {
        self.counters.keys_dropped.load(Ordering::SeqCst)
    }

    fn gateways_closed(&self) -> usize {
        self.counters.gateways_closed.load(Ordering::SeqCst)
    }

    fn open_attempts(&self) -> usize {
        self.counters.open_attempts.load(Ordering::SeqCst)
    }

    fn fail_next_open(&self, failure: Failure) {
        *self.connector.link.open_failure.lock().unwrap() = Some(failure);
    }

    fn set_activity(&self, in_flight: usize, last_request: Option<Instant>) {
        *self.connector.link.activity.lock().unwrap() = ProxyActivity {
            in_flight,
            last_request,
        };
    }

    fn spawn_connect(
        &self,
        run_label: &str,
    ) -> tokio::task::JoinHandle<Result<ModelEndpoint, ModelUnavailable>> {
        let connector = self.connector.clone();
        let interaction = Interaction::Allowed {
            run_label: Some(run_label.to_owned()),
        };
        tokio::spawn(async move { connector.connect(&target(MODEL), interaction).await })
    }

    async fn prompts_shown(&self, count: usize) -> Vec<UnlockPrompt> {
        for _ in 0..200 {
            let shown = self.shown();
            if shown.len() >= count {
                return shown;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("expected {count} prompt(s), saw {:?}", self.shown());
    }

    async fn unlock(&self, password: &str, keep: bool) -> Result<(), UnlockError> {
        self.connector
            .unlock(vault(password, keep), Unlocker::Prompt)
            .await
            .map(|_| ())
    }

    /// Unlocks `DEVICE` in the device area as `account`; true when the keys are kept.
    async fn area_unlock(&self, account: &str, keep: bool) -> Result<bool, UnlockError> {
        self.connector
            .unlock(vault_of(DEVICE, account, PASSWORD, keep), Unlocker::Area)
            .await
    }

    async fn forbidden(&self) -> Result<ModelEndpoint, ModelUnavailable> {
        self.connector
            .connect(&target(MODEL), Interaction::Forbidden)
            .await
    }
}

fn target(model: &str) -> DeviceModelTarget {
    target_on(DEVICE, model)
}

fn target_on(device_id: &str, model: &str) -> DeviceModelTarget {
    DeviceModelTarget {
        device_id: device_id.to_owned(),
        model: model.to_owned(),
        kind: DeviceModelKind::Chat,
        display_name: "Qwen3 8B".to_owned(),
    }
}

fn vault(password: &str, keep: bool) -> VaultUnlock {
    vault_of(DEVICE, ACCOUNT, password, keep)
}

fn vault_of(device_id: &str, account: &str, password: &str, keep: bool) -> VaultUnlock {
    serde_json::from_value(json!({
        "deviceId": device_id,
        "password": password,
        "controllerVault": [1, 2, 3],
        "manifestJws": "header.payload.signature",
        "grantId": "owner",
        "apiOrigin": "https://api.flow-like.test/",
        "account": account,
        "keepUnlocked": keep,
    }))
    .unwrap()
}

fn reason(result: Result<ModelEndpoint, ModelUnavailable>) -> ModelUnavailableReason {
    result.expect_err("the device must be unavailable").reason
}

#[tokio::test]
async fn locked_devices_answer_without_a_prompt_when_none_is_allowed() {
    let harness = Harness::new();
    let unavailable = harness.forbidden().await.unwrap_err();

    assert_eq!(
        unavailable.reason,
        ModelUnavailableReason::DeviceLockedDeclined
    );
    assert_eq!(
        unavailable.message,
        "Model 'Qwen3 8B' runs on device 'device-1', which stayed locked."
    );
    assert!(harness.shown().is_empty());
}

#[tokio::test]
async fn an_unlocked_prompt_serves_one_gateway_for_every_later_request() {
    let harness = Harness::new();
    let waiting = harness.spawn_connect("run-1");
    let prompt = harness.prompts_shown(1).await.remove(0);

    assert_eq!(prompt.device_id, DEVICE);
    assert_eq!(prompt.device_name, None);
    assert_eq!(prompt.model_name, "Qwen3 8B");
    assert_eq!(prompt.run_id.as_deref(), Some("run-1"));
    assert_eq!(prompt.run_name.as_deref(), Some("Daily summary"));
    assert_eq!(harness.connector.prompts.pending(), vec![prompt.clone()]);

    harness.unlock(PASSWORD, true).await.unwrap();
    let endpoint = waiting.await.unwrap().unwrap();
    assert_eq!(endpoint.base_url, "http://127.0.0.1:41000/v1");
    assert_eq!(endpoint.model, MODEL);
    assert_eq!(harness.closed(), vec![prompt.id]);
    assert!(harness.connector.prompts.pending().is_empty());

    let again = harness.forbidden().await.unwrap();
    assert_eq!(
        again, endpoint,
        "the same proxy keeps the factory cache warm"
    );
    assert_eq!(harness.counters.opened.load(Ordering::SeqCst), 1);
    assert_eq!(harness.shown().len(), 1);
}

#[tokio::test]
async fn a_wrong_password_keeps_the_prompt_and_a_decline_holds_for_the_run() {
    let harness = Harness::new();
    let waiting = harness.spawn_connect("run-1");
    let prompt = harness.prompts_shown(1).await.remove(0);

    let refused = harness.unlock("wrong", true).await.unwrap_err();
    assert_eq!(refused.code, "wrong_password");
    assert_eq!(harness.connector.prompts.pending().len(), 1);

    assert!(harness.connector.prompts.decline(&prompt.id));
    assert!(!harness.connector.prompts.decline(&prompt.id));
    assert_eq!(
        reason(waiting.await.unwrap()),
        ModelUnavailableReason::DeviceLockedDeclined
    );
    assert_eq!(harness.closed(), vec![prompt.id]);

    let same_run = harness.spawn_connect("run-1").await.unwrap();
    assert_eq!(
        reason(same_run),
        ModelUnavailableReason::DeviceLockedDeclined
    );
    let other_device = harness
        .connector
        .connect(
            &target_on(OTHER_DEVICE, MODEL),
            Interaction::Allowed {
                run_label: Some("run-1".to_owned()),
            },
        )
        .await;
    assert_eq!(
        reason(other_device),
        ModelUnavailableReason::DeviceLockedDeclined
    );
    assert_eq!(
        harness.shown().len(),
        1,
        "a declined run is not asked again, for any device"
    );

    let other_run = harness.spawn_connect("run-2");
    let second = harness.prompts_shown(2).await;
    assert_eq!(second[1].run_id.as_deref(), Some("run-2"));
    assert_eq!(second[1].run_name, None);
    harness.connector.prompts.decline(&second[1].id);
    other_run.await.unwrap().unwrap_err();
}

#[tokio::test]
async fn an_unanswered_prompt_times_out_as_a_decline() {
    let harness = Harness::new();
    let started = Instant::now();
    let result = harness.spawn_connect("run-1").await.unwrap();

    assert_eq!(reason(result), ModelUnavailableReason::DeviceLockedDeclined);
    assert!(started.elapsed() >= FAST.prompt);
    let prompt = harness.shown().remove(0);
    assert_eq!(harness.closed(), vec![prompt.id]);
    assert!(harness.connector.prompts.pending().is_empty());
}

#[tokio::test]
async fn without_a_window_the_answer_is_an_immediate_decline() {
    let harness = Harness::new();
    harness.window.hidden.store(true, Ordering::SeqCst);

    let result = harness.spawn_connect("run-1").await.unwrap();
    assert_eq!(reason(result), ModelUnavailableReason::DeviceLockedDeclined);
    assert!(harness.shown().is_empty());

    harness.window.hidden.store(false, Ordering::SeqCst);
    let result = harness.spawn_connect("run-1").await.unwrap();
    assert_eq!(reason(result), ModelUnavailableReason::DeviceLockedDeclined);
    assert!(harness.shown().is_empty(), "the decline holds for the run");
}

#[tokio::test]
async fn runs_waiting_for_the_same_device_share_one_prompt() {
    let harness = Harness::new();
    let first = harness.spawn_connect("run-1");
    let second = harness.spawn_connect("run-2");
    harness.prompts_shown(1).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(harness.shown().len(), 1);

    harness.unlock(PASSWORD, true).await.unwrap();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first, second);
    assert_eq!(harness.counters.opened.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn keys_lock_after_idling_unless_kept_until_quit() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, false).await.unwrap();
    harness.forbidden().await.unwrap();
    tokio::time::sleep(FAST.idle_lock * 3).await;

    assert_eq!(harness.dropped(), 1);
    assert_eq!(harness.gateways_closed(), 1);
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceLockedDeclined
    );

    harness.unlock(PASSWORD, true).await.unwrap();
    harness.forbidden().await.unwrap();
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 1);
    harness.forbidden().await.unwrap();
}

#[tokio::test]
async fn a_request_in_flight_keeps_unkept_keys_from_idling() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, false).await.unwrap();
    harness.forbidden().await.unwrap();
    harness.set_activity(1, Some(Instant::now()));
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 0, "a streaming answer is use");

    harness.set_activity(0, Some(Instant::now()));
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 1);
}

#[tokio::test]
async fn a_second_unlock_keeps_the_serving_keys_and_can_only_extend_them() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, false).await.unwrap();
    let served = harness.forbidden().await.unwrap();

    harness.unlock(PASSWORD, true).await.unwrap();
    assert_eq!(harness.dropped(), 1);
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.forbidden().await.unwrap(), served);
    assert_eq!(harness.dropped(), 1);
}

#[tokio::test]
async fn failures_name_the_model_the_device_and_the_cause() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();

    *harness.connector.link.open_failure.lock().unwrap() = Some(Failure::new(
        ModelUnavailableReason::NotGranted,
        "The device reset stream 3 (unauthorized): Use models is not granted.",
    ));
    let refused = harness.forbidden().await.unwrap_err();
    assert_eq!(refused.reason, ModelUnavailableReason::NotGranted);
    assert_eq!(
        refused.message,
        "Model 'Qwen3 8B' runs on device 'GPU box', which has not granted you model access \
         (The device reset stream 3 (unauthorized): Use models is not granted)."
    );

    let missing = harness
        .connector
        .connect(&target("llama-70b"), Interaction::Forbidden)
        .await
        .unwrap_err();
    assert_eq!(missing.reason, ModelUnavailableReason::ModelMissing);
    assert!(
        missing.message.contains("lists no model llama-70b"),
        "{}",
        missing.message
    );
}

#[tokio::test]
async fn a_failed_model_listing_fails_that_call_and_keeps_the_gateway_serving() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    let served = harness.forbidden().await.unwrap();

    *harness.connector.link.hosts_failure.lock().unwrap() = Some(Failure::new(
        ModelUnavailableReason::DeviceOffline,
        "its model gateway answered HTTP 502 Bad Gateway",
    ));
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceOffline
    );
    assert_eq!(
        harness.gateways_closed(),
        0,
        "requests in flight on the gateway finish"
    );

    assert_eq!(harness.forbidden().await.unwrap(), served);
    assert_eq!(harness.counters.opened.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_offline_device_answers_at_once_until_it_is_worth_trying_again() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    let offline = || {
        Failure::new(
            ModelUnavailableReason::DeviceOffline,
            "no open data channel within 15s",
        )
    };

    harness.fail_next_open(offline());
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceOffline
    );
    let again = harness.forbidden().await.unwrap_err();
    assert_eq!(again.reason, ModelUnavailableReason::DeviceOffline);
    assert!(
        again.message.contains("no open data channel"),
        "{}",
        again.message
    );
    assert_eq!(harness.open_attempts(), 1, "no second connect right away");

    tokio::time::sleep(FAST.retry_offline * 2).await;
    harness.fail_next_open(offline());
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceOffline
    );
    assert_eq!(harness.open_attempts(), 2);

    harness.unlock(PASSWORD, true).await.unwrap();
    harness.forbidden().await.unwrap();
    assert_eq!(
        harness.open_attempts(),
        3,
        "a new unlock tries again at once"
    );
}

#[tokio::test]
async fn a_lock_does_not_wait_for_a_slow_connect_and_the_late_gateway_is_dropped() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    *harness.connector.link.open_delay.lock().unwrap() = Some(Duration::from_millis(300));
    let connecting = {
        let connector = harness.connector.clone();
        tokio::spawn(async move {
            connector
                .connect(&target(MODEL), Interaction::Forbidden)
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(harness.open_attempts(), 1);

    let started = Instant::now();
    harness.connector.lock(DEVICE).await;
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "the lock waited for the connect"
    );
    assert_eq!(
        harness.counters.keys_closed.load(Ordering::SeqCst),
        1,
        "the keys close although the connect still holds them"
    );
    assert!(harness.connector.held().is_empty());

    assert_eq!(
        reason(connecting.await.unwrap()),
        ModelUnavailableReason::DeviceLockedDeclined
    );
    assert_eq!(harness.counters.opened.load(Ordering::SeqCst), 1);
    assert_eq!(
        harness.gateways_closed(),
        1,
        "the gateway opened for the locked keys is dropped"
    );
    assert_eq!(harness.dropped(), 1);
}

#[tokio::test]
async fn locking_one_device_drops_its_kept_keys_and_gateway_only() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    harness.forbidden().await.unwrap();

    harness.connector.lock(OTHER_DEVICE).await;
    assert_eq!(harness.dropped(), 0);

    harness.connector.lock(DEVICE).await;
    assert_eq!(harness.dropped(), 1);
    assert_eq!(harness.gateways_closed(), 1);
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceLockedDeclined
    );
}

#[tokio::test]
async fn the_area_letting_go_leaves_kept_keys_and_a_lock_drops_them() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    let served = harness.forbidden().await.unwrap();

    assert!(
        harness.area_unlock(ACCOUNT, false).await.unwrap(),
        "a keep from the prompt survives an area unlock without one"
    );
    assert_eq!(harness.dropped(), 1, "the serving keys stay");
    harness.connector.release(DEVICE).await;
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.forbidden().await.unwrap(), served);
    assert_eq!(harness.dropped(), 1);

    harness.area_unlock(ACCOUNT, false).await.unwrap();
    harness.connector.lock(DEVICE).await;
    assert_eq!(harness.dropped(), 3);
    assert_eq!(harness.gateways_closed(), 1);
    assert!(harness.connector.held().is_empty());
}

#[tokio::test]
async fn the_area_letting_go_locks_unused_keys_and_waits_for_runs_using_them() {
    let harness = Harness::new();
    harness.area_unlock(ACCOUNT, false).await.unwrap();
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 0, "the area decides while it holds them");
    harness.connector.release(DEVICE).await;
    assert_eq!(harness.dropped(), 1, "nothing used them, so they lock now");

    harness.area_unlock(ACCOUNT, false).await.unwrap();
    harness.forbidden().await.unwrap();
    harness.set_activity(1, Some(Instant::now()));
    harness.connector.release(DEVICE).await;
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 1, "a run is streaming an answer");
    harness.forbidden().await.unwrap();

    harness.set_activity(0, Some(Instant::now()));
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 2, "the keys idle out after the run");
    assert_eq!(harness.gateways_closed(), 1);
}

#[tokio::test]
async fn keeping_in_the_area_holds_the_keys_after_it_lets_go() {
    let harness = Harness::new();
    assert!(!harness.area_unlock(ACCOUNT, false).await.unwrap());
    harness.connector.keep(DEVICE, true).await;
    harness.connector.release(DEVICE).await;
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 0);

    harness.connector.keep(DEVICE, false).await;
    tokio::time::sleep(FAST.idle_lock * 3).await;
    assert_eq!(harness.dropped(), 1, "unkept keys idle out on their own");
}

#[tokio::test]
async fn another_account_replaces_the_keys_and_tunnel_of_the_first() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    let first = harness.forbidden().await.unwrap();

    assert!(!harness.area_unlock(OTHER_ACCOUNT, false).await.unwrap());
    assert_eq!(harness.dropped(), 1, "the first account's keys are gone");
    assert_eq!(harness.gateways_closed(), 1);
    let held = harness.connector.held();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].account, OTHER_ACCOUNT);
    assert!(!held[0].kept);

    let second = harness.forbidden().await.unwrap();
    assert_ne!(second, first);
    assert_eq!(harness.counters.opened.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn signed_out_accounts_lose_their_keys_and_cannot_unlock() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    harness.forbidden().await.unwrap();

    harness.connector.drop_signed_out().await;
    assert_eq!(harness.dropped(), 0, "the account is still signed in");

    harness
        .connector
        .link
        .signed_out
        .lock()
        .unwrap()
        .insert(ACCOUNT.to_owned());
    harness.connector.drop_signed_out().await;
    assert_eq!(harness.dropped(), 1);
    assert_eq!(harness.gateways_closed(), 1);
    assert_eq!(
        reason(harness.forbidden().await),
        ModelUnavailableReason::DeviceLockedDeclined
    );

    let refused = harness.unlock(PASSWORD, true).await.unwrap_err();
    assert_eq!(refused.code, "signed_out");
    assert_eq!(harness.dropped(), 2, "the opened keys are not kept");
    assert!(harness.connector.held().is_empty());
}

#[tokio::test]
async fn locking_every_device_also_stops_an_unlock_in_flight() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    harness
        .connector
        .unlock(
            vault_of(OTHER_DEVICE, ACCOUNT, PASSWORD, false),
            Unlocker::Prompt,
        )
        .await
        .unwrap();
    assert_eq!(harness.connector.held().len(), 2);

    *harness.connector.link.unlock_delay.lock().unwrap() = Some(Duration::from_millis(80));
    let unlocking = {
        let connector = harness.connector.clone();
        tokio::spawn(async move {
            connector
                .unlock(vault_of(DEVICE, ACCOUNT, PASSWORD, true), Unlocker::Area)
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(10)).await;
    harness.connector.lock_all().await;
    assert_eq!(unlocking.await.unwrap().unwrap_err().code, "locked");
    assert_eq!(harness.dropped(), 3);
    assert!(harness.connector.held().is_empty());
}

#[tokio::test]
async fn every_change_of_the_held_keys_is_published() {
    let harness = Harness::new();
    harness.unlock(PASSWORD, true).await.unwrap();
    harness.forbidden().await.unwrap();
    harness.area_unlock(ACCOUNT, false).await.unwrap();
    harness.connector.release(DEVICE).await;
    harness.connector.lock(DEVICE).await;

    let held = |kept: bool, area: bool| {
        vec![HeldKeys {
            device_id: DEVICE.to_owned(),
            device_name: Some("GPU box".to_owned()),
            api_origin: "https://api.flow-like.test".to_owned(),
            account: ACCOUNT.to_owned(),
            kept,
            area,
        }]
    };
    assert_eq!(
        *harness.window.held.lock().unwrap(),
        vec![
            held(true, false),
            held(true, true),
            held(true, false),
            vec![]
        ],
        "a use that changes nothing is not published"
    );
}

#[test]
fn unlock_requests_follow_the_webview_contract() {
    let request = vault(PASSWORD, true);
    assert_eq!(request.device_id, DEVICE);
    assert_eq!(request.controller_vault, vec![1, 2, 3]);
    assert!(request.owner_controller_key.is_none());
    assert_eq!(
        Holder::of(&request),
        Holder {
            api_origin: "https://api.flow-like.test".to_owned(),
            account: ACCOUNT.to_owned(),
            grant_id: "owner".to_owned(),
        }
    );

    let extra = serde_json::from_value::<VaultUnlock>(json!({
        "deviceId": DEVICE,
        "password": PASSWORD,
        "controllerVault": [1],
        "manifestJws": "a.b.c",
        "grantId": "grant-1",
        "ownerControllerKey": {"kty": "OKP", "crv": "Ed25519", "x": "A".repeat(43)},
        "apiOrigin": "https://api.flow-like.test",
        "account": "user-1",
        "keepUnlocked": false,
        "controllerSeed": "never sent",
    }));
    assert!(extra.is_err(), "only the password and the vault cross over");
}
