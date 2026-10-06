use std::fmt;

use async_trait::async_trait;
use flow_like::models::device::{ModelEndpoint, ModelUnavailableReason};
use flow_like_device_client::ProxyActivity;
use flow_like_device_protocol::Ed25519PublicKey;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// What the webview hands over to unlock one device: the password and the encrypted controller
/// vault from its device storage, the public fields of that vault, and the account whose hub
/// session reaches the device. The decrypted keys never leave this process.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VaultUnlock {
    pub(crate) device_id: String,
    pub(crate) password: Password,
    pub(crate) controller_vault: Vec<u8>,
    pub(crate) manifest_jws: String,
    pub(crate) grant_id: String,
    #[serde(default)]
    pub(crate) owner_controller_key: Option<Ed25519PublicKey>,
    pub(crate) api_origin: String,
    pub(crate) account: String,
    pub(crate) keep_unlocked: bool,
}

/// The hub account and grant whose authority unlocked keys carry. Keys of one holder never
/// serve another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holder {
    pub(crate) api_origin: String,
    pub(crate) account: String,
    pub(crate) grant_id: String,
}

impl Holder {
    pub(crate) fn of(vault: &VaultUnlock) -> Self {
        Self {
            api_origin: vault.api_origin.trim_end_matches('/').to_owned(),
            account: vault.account.clone(),
            grant_id: vault.grant_id.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(crate) struct Password(String);

impl Password {
    pub(crate) fn into_bytes(mut self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(std::mem::take(&mut self.0).into_bytes())
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Why a device cannot serve a model right now; `detail` names the cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) reason: ModelUnavailableReason,
    pub(crate) detail: String,
}

impl Failure {
    pub(crate) fn new(reason: ModelUnavailableReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }

    pub(crate) fn locked() -> Self {
        Self::new(ModelUnavailableReason::DeviceLockedDeclined, "")
    }
}

/// A refused unlock as the prompt shows it: `code` picks the sentence, `message` says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnlockError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl UnlockError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for UnlockError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// An opened vault: the controller authority over the device and the name its identity carries.
pub(crate) struct Unlocked<K> {
    pub(crate) keys: K,
    pub(crate) device_name: String,
}

/// What the unlock dialog shows. Without an answer by `expires_at` (Unix milliseconds) the
/// prompt counts as declined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnlockPrompt {
    pub(crate) id: String,
    pub(crate) device_id: String,
    pub(crate) device_name: Option<String>,
    pub(crate) model_name: String,
    pub(crate) run_id: Option<String>,
    pub(crate) run_name: Option<String>,
    pub(crate) expires_at: u64,
}

/// The transport under the connector; tests replace it.
#[async_trait]
pub(crate) trait DeviceLink: Send + Sync + 'static {
    /// Dropping them zeroizes the controller keys and closes their tunnels.
    type Keys: Send + Sync + 'static;
    /// Dropping it stops its loopback proxy; the port stays bound for the device's next one.
    type Gateway: Send + Sync + 'static;

    /// Opens the vault and verifies the device identity it is pinned to.
    async fn unlock(&self, vault: VaultUnlock) -> Result<Unlocked<Self::Keys>, UnlockError>;

    /// Zeroizes the keys and ends their tunnel now, also while a connect still holds them.
    fn close(&self, keys: &Self::Keys);

    /// Connects to the device and serves its model gateway on loopback.
    async fn open(&self, keys: &Self::Keys) -> Result<Self::Gateway, Failure>;

    async fn hosts(&self, gateway: &Self::Gateway, model: &str) -> Result<bool, Failure>;

    fn endpoint(&self, gateway: &Self::Gateway, model: &str) -> ModelEndpoint;

    /// Requests the gateway serves now and when one last ran; a streamed answer counts until
    /// its body ends.
    fn activity(&self, gateway: &Self::Gateway) -> ProxyActivity;

    /// A window is still signed in to the hub account of `holder`.
    fn signed_in(&self, holder: &Holder) -> bool;
}

/// A device whose keys this process holds, as the device area and the tray list it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeldKeys {
    pub(crate) device_id: String,
    pub(crate) device_name: Option<String>,
    pub(crate) api_origin: String,
    pub(crate) account: String,
    /// Kept for model access: only a lock, a sign-out or quitting drops them.
    pub(crate) kept: bool,
    /// The device area of the main window holds them too.
    pub(crate) area: bool,
}

/// Where unlock prompts appear: the main window, or a fake in tests.
pub(crate) trait PromptHost: Send + Sync + 'static {
    /// False without a visible window; the answer is then an immediate decline.
    fn can_prompt(&self) -> bool;

    fn show(&self, prompt: &UnlockPrompt);

    fn close(&self, prompt_id: &str);

    /// The event or board a run started from, for the prompt.
    fn run_name(&self, run_label: &str) -> Option<String>;

    /// Every device whose keys are held now, after each change.
    fn held(&self, held: &[HeldKeys]);
}
