use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex, PoisonError, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use flow_like::models::device::{ModelEndpoint, ModelUnavailableReason};
use flow_like_device_client::{
    AccessToken, ControllerKeys, DeviceClient, DeviceSession, DeviceTarget, Error as LinkError,
    HttpHubClient, LoopbackPort, LoopbackProxy, ProxyActivity, TunnelDataOpen, TunnelMode,
    TunnelOpen, TunnelTarget,
};
use flow_like_device_crypto::controller::verify_device_receipt;
use flow_like_device_protocol::{
    DeviceReceipt, Ed25519PublicKey, ManagementCommand, ManagementRequest, ManagementResponse,
    OnboardingManifest, validate_management_id,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::Instant,
};

use super::link::{DeviceLink, Failure, Holder, UnlockError, Unlocked, VaultUnlock};

const HUB_TIMEOUT: Duration = Duration::from_secs(15);
const GATEWAY_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_INSPECTION_BYTES: u64 = 1 << 20;
const MAX_ERROR_CHARS: usize = 512;
const MODEL_LIST_TTL: Duration = Duration::from_secs(30);
const OWNER_GRANT: &str = "owner";
/// The `Host` the model gateway of the device sees.
const GATEWAY_HOST: &str = "localhost";

static HELD: LazyLock<Mutex<Vec<Weak<ControllerKeys>>>> = LazyLock::new(Mutex::default);

/// Zeroizes every controller key this process unlocked.
pub(crate) fn lock_all() {
    let held = std::mem::take(&mut *HELD.lock().unwrap_or_else(PoisonError::into_inner));
    for keys in held.iter().filter_map(Weak::upgrade) {
        keys.lock();
    }
}

fn hold(keys: &Arc<ControllerKeys>) {
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    held.retain(|keys| keys.strong_count() > 0);
    held.push(Arc::downgrade(keys));
}

/// The account whose hub session reaches the device, and the vault fields its identity is
/// checked against.
struct Authority {
    device_id: String,
    api_origin: String,
    account: String,
    manifest_jws: String,
    grant_id: String,
    owner_key: Option<Ed25519PublicKey>,
}

impl Authority {
    fn api_base(&self) -> String {
        format!("{}/api/v1", self.api_origin.trim_end_matches('/'))
    }

    fn is_owner(&self) -> bool {
        self.grant_id == OWNER_GRANT
    }

    /// The key the onboarding manifest is pinned to: the own controller key of an owner vault,
    /// the owner key a shared vault carries.
    fn anchor(&self, own: Ed25519PublicKey) -> Result<Ed25519PublicKey, String> {
        match (&self.owner_key, self.is_owner()) {
            (None, true) => Ok(own),
            (Some(key), false) if *key != own => Ok(key.clone()),
            _ => Err(format!(
                "the vault of device {} mixes owner and shared-access authority (grant {})",
                self.device_id, self.grant_id
            )),
        }
    }

    fn binds(&self, manifest: &OnboardingManifest, receipt: &DeviceReceipt) -> bool {
        manifest.device_id == self.device_id
            && receipt.device_id == self.device_id
            && manifest.api_base_url == self.api_base()
            && (!self.is_owner() || manifest.owner_id == self.account)
    }
}

pub(crate) struct NativeKeys {
    keys: Arc<ControllerKeys>,
    authority: Authority,
    /// The one tunnel to the device, shared by model calls, local ports and transfers.
    session: tokio::sync::Mutex<Option<DeviceSession>>,
}

impl Drop for NativeKeys {
    fn drop(&mut self) {
        if let Some(session) = self.session.get_mut().take() {
            session.close();
        }
        self.keys.lock();
    }
}

pub(crate) struct NativeGateway {
    proxy: LoopbackProxy,
    listed: Mutex<Option<(Instant, HashSet<String>)>>,
}

impl NativeGateway {
    fn lists(&self, model: &str) -> bool {
        self.listed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(at, models)| at.elapsed() < MODEL_LIST_TTL && models.contains(model))
    }

    fn remember(&self, models: HashSet<String>) {
        *self.listed.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), models));
    }
}

impl Drop for NativeGateway {
    fn drop(&mut self) {
        self.proxy.close();
    }
}

/// Why the hub or the identity of the device refused, before any tunnel exists.
enum Refusal {
    SignedOut(String),
    Unavailable(String),
    Unreachable(String),
    Mismatch(String),
}

impl Refusal {
    fn unlock_error(self) -> UnlockError {
        match self {
            Self::SignedOut(message) => UnlockError::new("signed_out", message),
            Self::Unavailable(message) => UnlockError::new("device_unavailable", message),
            Self::Unreachable(message) => UnlockError::new("hub_unreachable", message),
            Self::Mismatch(message) => UnlockError::new("authority_mismatch", message),
        }
    }

    fn failure(self) -> Failure {
        match self {
            Self::SignedOut(message) | Self::Unreachable(message) => {
                Failure::new(ModelUnavailableReason::DeviceOffline, message)
            }
            Self::Unavailable(message) | Self::Mismatch(message) => {
                Failure::new(ModelUnavailableReason::DeviceRemoved, message)
            }
        }
    }
}

/// The hub session of the account signed in to the main window; read for every admission, so
/// refreshed tokens apply.
struct SessionToken {
    hub: String,
    account: String,
    device_id: String,
}

#[async_trait]
impl AccessToken for SessionToken {
    async fn access_token(&self) -> flow_like_device_client::Result<String> {
        crate::execution_credentials::session_token(&self.hub, &self.account).ok_or_else(|| {
            LinkError::Unreachable {
                device_id: self.device_id.clone(),
                message: format!(
                    "no window is signed in to {} as account {}; sign in again",
                    self.hub, self.account
                ),
            }
        })
    }
}

pub(crate) struct NativeLink {
    hub: reqwest::Client,
    local: reqwest::Client,
    /// One loopback port per device while the app runs. A closed gateway keeps its port, so a
    /// client that kept the address reaches no other process that could take it over.
    ports: Mutex<HashMap<String, Arc<LoopbackPort>>>,
}

impl NativeLink {
    pub(crate) fn new() -> Self {
        Self {
            hub: reqwest::Client::builder()
                .connect_timeout(HUB_TIMEOUT)
                .build()
                .unwrap_or_default(),
            local: reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_default(),
            ports: Mutex::default(),
        }
    }

    /// The port of the device's model gateway, bound on first use.
    async fn port(&self, device_id: &str) -> Result<Arc<LoopbackPort>, Failure> {
        let bound = self
            .ports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(device_id)
            .cloned();
        if let Some(port) = bound {
            return Ok(port);
        }
        let port = LoopbackPort::bind(GATEWAY_HOST)
            .await
            .map_err(link_failure)?;
        Ok(self
            .ports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(device_id.to_owned())
            .or_insert(port)
            .clone())
    }

    async fn receipt(&self, keys: &NativeKeys) -> Result<DeviceReceipt, Refusal> {
        let authority = &keys.authority;
        let token =
            crate::execution_credentials::session_token(&authority.api_origin, &authority.account)
                .ok_or_else(|| {
                    Refusal::SignedOut(format!(
                        "no window is signed in to {} as account {}",
                        authority.api_origin, authority.account
                    ))
                })?;
        let url = format!(
            "{}/devices/{}/identity",
            authority.api_base(),
            authority.device_id
        );
        let response = self
            .hub
            .get(&url)
            .bearer_auth(token)
            .timeout(HUB_TIMEOUT)
            .send()
            .await
            .map_err(|error| Refusal::Unreachable(format!("GET {url} failed: {error}")))?;
        let status = response.status();
        match status.as_u16() {
            401 => {
                return Err(Refusal::SignedOut(format!(
                    "GET {url} answered HTTP {status}; sign in again"
                )));
            }
            403 | 404 => {
                return Err(Refusal::Unavailable(format!(
                    "GET {url} answered HTTP {status}"
                )));
            }
            _ if !status.is_success() => {
                return Err(Refusal::Unreachable(format!(
                    "GET {url} answered HTTP {status}"
                )));
            }
            _ => {}
        }
        let receipt = response.json::<DeviceReceipt>().await.map_err(|error| {
            Refusal::Unreachable(format!("the identity from GET {url} is malformed: {error}"))
        })?;
        verify(keys, &receipt).map_err(Refusal::Mismatch)?;
        Ok(receipt)
    }

    async fn list_models(&self, gateway: &NativeGateway) -> Result<HashSet<String>, Failure> {
        let url = format!("{}/v1/models", gateway.proxy.base_url());
        let response = self
            .local
            .get(&url)
            .bearer_auth(gateway.proxy.bearer())
            .timeout(GATEWAY_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                Failure::new(
                    ModelUnavailableReason::DeviceOffline,
                    format!("listing its models failed: {error}"),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            let reason = match status.as_u16() {
                401 | 403 => ModelUnavailableReason::NotGranted,
                _ => ModelUnavailableReason::DeviceOffline,
            };
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.trim().chars().take(MAX_ERROR_CHARS).collect();
            return Err(Failure::new(
                reason,
                format!("its model gateway answered HTTP {status}: {detail}"),
            ));
        }
        let list = response.json::<ModelList>().await.map_err(|error| {
            Failure::new(
                ModelUnavailableReason::DeviceOffline,
                format!("its model list is malformed: {error}"),
            )
        })?;
        Ok(list.data.into_iter().map(|model| model.id).collect())
    }

    /// The device's session, connected on first use. It lives as long as the keys: a lost tunnel
    /// is replaced on the next open, and locking closes it.
    pub(crate) async fn session(&self, keys: &NativeKeys) -> Result<DeviceSession, Failure> {
        let mut slot = keys.session.lock().await;
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let receipt = self.receipt(keys).await.map_err(Refusal::failure)?;
        let authority = &keys.authority;
        let token = SessionToken {
            hub: authority.api_origin.clone(),
            account: authority.account.clone(),
            device_id: authority.device_id.clone(),
        };
        let hub =
            HttpHubClient::new(&authority.api_base(), Arc::new(token)).map_err(link_failure)?;
        let target = DeviceTarget::from_receipt(&receipt, authority.grant_id.clone());
        let session = DeviceClient::new(Arc::new(hub))
            .connect(target, keys.keys.clone())
            .await
            .map_err(link_failure)?;
        *slot = Some(session.clone());
        Ok(session)
    }
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
}

#[async_trait]
impl DeviceLink for NativeLink {
    type Keys = NativeKeys;
    type Gateway = NativeGateway;

    async fn unlock(&self, vault: VaultUnlock) -> Result<Unlocked<NativeKeys>, UnlockError> {
        let VaultUnlock {
            device_id,
            password,
            controller_vault,
            manifest_jws,
            grant_id,
            owner_controller_key,
            api_origin,
            account,
            keep_unlocked: _,
        } = vault;
        let authority = Authority {
            device_id,
            api_origin,
            account,
            manifest_jws,
            grant_id,
            owner_key: owner_controller_key,
        };
        validate(&authority)?;
        let keys = ControllerKeys::unlock(
            authority.device_id.clone(),
            password.into_bytes(),
            controller_vault,
        )
        .await
        .map_err(|error| UnlockError::new("wrong_password", error.to_string()))?;
        hold(&keys);
        let keys = NativeKeys {
            keys,
            authority,
            session: tokio::sync::Mutex::default(),
        };
        let receipt = self.receipt(&keys).await.map_err(Refusal::unlock_error)?;
        Ok(Unlocked {
            keys,
            device_name: receipt.name,
        })
    }

    /// Locked controller keys end every tunnel they certified, and a connect still running
    /// fails on them.
    fn close(&self, keys: &NativeKeys) {
        keys.keys.lock();
    }

    async fn open(&self, keys: &NativeKeys) -> Result<NativeGateway, Failure> {
        let session = self.session(keys).await?;
        let port = self.port(&keys.authority.device_id).await?;
        Ok(NativeGateway {
            proxy: serve_gateway(&port, session).await?,
            listed: Mutex::new(None),
        })
    }

    async fn hosts(&self, gateway: &NativeGateway, model: &str) -> Result<bool, Failure> {
        if gateway.lists(model) {
            return Ok(true);
        }
        let models = self.list_models(gateway).await?;
        let hosted = models.contains(model);
        gateway.remember(models);
        Ok(hosted)
    }

    fn endpoint(&self, gateway: &NativeGateway, model: &str) -> ModelEndpoint {
        ModelEndpoint {
            base_url: format!("{}/v1", gateway.proxy.base_url()),
            bearer: gateway.proxy.bearer().to_owned(),
            model: model.to_owned(),
        }
    }

    fn activity(&self, gateway: &NativeGateway) -> ProxyActivity {
        gateway.proxy.activity()
    }

    fn signed_in(&self, holder: &Holder) -> bool {
        crate::execution_credentials::session_token(&holder.api_origin, &holder.account).is_some()
    }
}

fn validate(authority: &Authority) -> Result<(), UnlockError> {
    let origin = reqwest::Url::parse(&authority.api_origin).ok();
    let valid = validate_management_id(&authority.device_id).is_ok()
        && validate_management_id(&authority.grant_id).is_ok()
        && !authority.account.trim().is_empty()
        && !authority.manifest_jws.is_empty()
        && origin.is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    if valid {
        Ok(())
    } else {
        Err(UnlockError::new(
            "invalid",
            format!(
                "the unlock request for device {} names an invalid device, grant, account or hub {}",
                authority.device_id, authority.api_origin
            ),
        ))
    }
}

/// The checks the device area makes when it unlocks (`verifyIdentity`, `assertVaultAuthority`):
/// the receipt is bound to the pinned onboarding manifest, and that manifest to this hub, this
/// account and the authority of the vault.
fn verify(keys: &NativeKeys, receipt: &DeviceReceipt) -> Result<(), String> {
    let authority = &keys.authority;
    let own = keys
        .keys
        .controller_key()
        .map_err(|error| error.to_string())?;
    let anchor = authority.anchor(own)?;
    let manifest =
        verify_device_receipt(receipt, &authority.manifest_jws, &anchor).map_err(|error| {
            format!(
                "the identity of device {} does not match its vault: {error:#}",
                authority.device_id
            )
        })?;
    if authority.binds(&manifest, receipt) {
        Ok(())
    } else {
        Err(format!(
            "the signed identity of device {} belongs to another device, hub or account",
            authority.device_id
        ))
    }
}

fn gateway_open() -> TunnelOpen {
    TunnelOpen {
        placement_id: String::new(),
        service_id: String::new(),
        mode: TunnelMode::Http,
        target: TunnelTarget::ModelGateway,
    }
}

/// Agents without `model_host` cannot decode a model-gateway open and close the whole tunnel,
/// so the flag is read first. A grant without Status cannot read it; opening then risks only
/// this tunnel, and a grant holding Use models was accepted only by an agent that decodes the
/// gateway target. The first open names a refusal before any request depends on it.
async fn serve_gateway(
    port: &Arc<LoopbackPort>,
    session: DeviceSession,
) -> Result<LoopbackProxy, Failure> {
    if hosts_models(&session).await? == Some(false) {
        return Err(Failure::new(
            ModelUnavailableReason::ModelMissing,
            "its agent does not host models; update the agent",
        ));
    }
    drop(
        session
            .open_stream(gateway_open())
            .await
            .map_err(link_failure)?,
    );
    port.serve(session, gateway_open()).map_err(link_failure)
}

/// `None` when this grant may not inspect the device.
async fn hosts_models(session: &DeviceSession) -> Result<Option<bool>, Failure> {
    let issued_at = unix_now();
    let request = ManagementRequest {
        operation_id: format!("model-host-{}", uuid::Uuid::new_v4()),
        device_id: session.device_id().to_owned(),
        issued_at,
        expires_at: issued_at + 60,
        command: ManagementCommand::Inspect,
    };
    let mut stream = session
        .open_data(TunnelDataOpen::Request { request })
        .await
        .map_err(link_failure)?;
    let mut reply = Vec::new();
    let read = async {
        stream.shutdown().await?;
        (&mut stream)
            .take(MAX_INSPECTION_BYTES)
            .read_to_end(&mut reply)
            .await
    };
    read.await.map_err(|error| {
        Failure::new(
            ModelUnavailableReason::DeviceOffline,
            format!("reading its inspection failed: {error}"),
        )
    })?;
    let response = serde_json::from_slice::<ManagementResponse>(&reply).map_err(|error| {
        Failure::new(
            ModelUnavailableReason::DeviceOffline,
            format!("its inspection is malformed: {error}"),
        )
    })?;
    match response.state.as_str() {
        "completed" => Ok(Some(
            response.result["features"]["model_host"].as_u64() == Some(1),
        )),
        "rejected" if response.result["code"] == "unauthorized" => Ok(None),
        state => Err(Failure::new(
            ModelUnavailableReason::DeviceOffline,
            format!(
                "it answered its inspection with {state}: {}",
                response.result["error"]
            ),
        )),
    }
}

fn link_failure(error: LinkError) -> Failure {
    let reason = match &error {
        LinkError::Refused {
            status: Some(403 | 404),
            ..
        } => ModelUnavailableReason::DeviceRemoved,
        LinkError::Reset { code, .. } if code == "unauthorized" => {
            ModelUnavailableReason::NotGranted
        }
        LinkError::Reset { code, .. } if code == "unsupported" => {
            ModelUnavailableReason::ModelMissing
        }
        LinkError::Locked { .. } | LinkError::Unlock { .. } => {
            ModelUnavailableReason::DeviceLockedDeclined
        }
        _ => ModelUnavailableReason::DeviceOffline,
    };
    Failure::new(reason, error.to_string())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}
