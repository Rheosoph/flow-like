use crate::{
    controller, mls::MemberIdentity, mls_store::MlsPins, noise, prepared_mls::PreparedMlsEndpoint,
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{Ed25519PublicKey, SigningKey, TelemetryMember};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;

fn error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}
fn encode(value: &impl Serialize) -> Result<JsValue> {
    Ok(serde_wasm_bindgen::to_value(value).map_err(error_any)?)
}
fn decode<T: DeserializeOwned>(value: JsValue) -> Result<T> {
    Ok(serde_wasm_bindgen::from_value(value).map_err(error_any)?)
}
fn error_any(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}
fn time(now: f64) -> Result<i64> {
    ensure!(
        now.is_finite() && now > 0.0 && now <= 9_007_199_254_740_991.0 && now.fract() == 0.0,
        "Invalid current time"
    );
    Ok(now as i64)
}

trait Closeable {
    fn close(&self);
}
struct Managed<T>(RefCell<Option<T>>);
impl<T> Closeable for Managed<T> {
    fn close(&self) {
        self.0.borrow_mut().take();
    }
}
#[derive(Default)]
struct Registry {
    closed: Cell<bool>,
    children: RefCell<Vec<Weak<dyn Closeable>>>,
}
impl Registry {
    fn register<T: 'static>(self: &Rc<Self>, value: T) -> Result<Handle<T>> {
        ensure!(!self.closed.get(), "Controller is locked");
        let inner = Rc::new(Managed(RefCell::new(Some(value))));
        let erased: Rc<dyn Closeable> = inner.clone();
        let mut children = self.children.borrow_mut();
        children.retain(|child| child.strong_count() > 0);
        ensure!(children.len() < 64, "Too many unlocked controller sessions");
        children.push(Rc::downgrade(&erased));
        Ok(Handle {
            inner,
            registry: self.clone(),
        })
    }
    fn close(&self) {
        self.closed.set(true);
        for child in self
            .children
            .borrow_mut()
            .drain(..)
            .filter_map(|child| child.upgrade())
        {
            child.close();
        }
    }
}
struct Handle<T> {
    inner: Rc<Managed<T>>,
    registry: Rc<Registry>,
}
impl<T> Handle<T> {
    fn with<R>(&self, operation: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        ensure!(!self.registry.closed.get(), "Controller is locked");
        let mut inner = self.inner.0.borrow_mut();
        operation(inner.as_mut().context("Cryptographic session is closed")?)
    }
    fn take(&self) -> Result<T> {
        ensure!(!self.registry.closed.get(), "Controller is locked");
        self.inner
            .0
            .borrow_mut()
            .take()
            .context("Cryptographic session is closed")
    }
    fn close(&self) {
        self.inner.close();
    }
}

/// Password parameters are owned Uint8Array values. Rust clears its copied
/// bytes; the UI clears its original input and buffer after each call.
#[wasm_bindgen(js_name = createControllerVault)]
pub fn create_controller_vault(
    device_id: &str,
    password: Vec<u8>,
) -> std::result::Result<JsValue, JsValue> {
    let password = Zeroizing::new(password);
    controller::create_controller_vault(device_id, &password)
        .and_then(|vault| encode(&vault))
        .map_err(error)
}

#[wasm_bindgen(js_name = createInvitationVault)]
pub fn create_invitation_vault(
    device_id: &str,
    password: Vec<u8>,
) -> std::result::Result<JsValue, JsValue> {
    let password = Zeroizing::new(password);
    controller::create_invitation_vault(device_id, &password)
        .and_then(|vault| encode(&vault))
        .map_err(error)
}

#[wasm_bindgen(js_name = createOnboardingVaults)]
pub fn create_onboarding_vaults(password: Vec<u8>) -> std::result::Result<JsValue, JsValue> {
    let password = Zeroizing::new(password);
    controller::create_onboarding_vaults(&password)
        .and_then(|vaults| encode(&vaults))
        .map_err(error)
}

/// This explicit export is only for the expiring enrollment bootstrap credential
/// inside the target package. Controller and invitation seeds have no export API.
#[wasm_bindgen(js_name = createBootstrapKey)]
pub fn create_bootstrap_key() -> std::result::Result<JsValue, JsValue> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let key = SigningKey::generate();
    let seed = Zeroizing::new(key.to_bytes());
    let encoded = Zeroizing::new(URL_SAFE_NO_PAD.encode(seed.as_ref()));
    #[derive(Serialize)]
    struct Bootstrap<'a> {
        public_key: Ed25519PublicKey,
        secret_base64: &'a str,
    }
    encode(&Bootstrap {
        public_key: key.public_key(),
        secret_base64: &encoded,
    })
    .map_err(error)
}

#[wasm_bindgen(js_name = unlockControllerVault)]
pub fn unlock_controller_vault(
    device_id: &str,
    password: Vec<u8>,
    ciphertext: &[u8],
) -> std::result::Result<BrowserController, JsValue> {
    let password = Zeroizing::new(password);
    let controller =
        controller::unlock_controller_vault(device_id, &password, ciphertext).map_err(error)?;
    Ok(BrowserController {
        controller: Some(controller),
        registry: Rc::new(Registry::default()),
    })
}

#[wasm_bindgen(js_name = signManagementPolicy)]
pub fn sign_management_policy(
    policy: JsValue,
    password: Vec<u8>,
    ciphertext: &[u8],
) -> std::result::Result<String, JsValue> {
    let password = Zeroizing::new(password);
    decode(policy)
        .and_then(|policy| controller::approve_management_policy(&policy, &password, ciphertext))
        .map_err(error)
}

/// Empty invitation_ciphertext means this user has no local invitation authority.
#[wasm_bindgen(js_name = rewrapControllerVaults)]
pub fn rewrap_controller_vaults(
    device_id: &str,
    current_password: Vec<u8>,
    new_password: Vec<u8>,
    controller_ciphertext: &[u8],
    invitation_ciphertext: &[u8],
) -> std::result::Result<JsValue, JsValue> {
    let current_password = Zeroizing::new(current_password);
    let new_password = Zeroizing::new(new_password);
    controller::rewrap_controller_vaults(
        device_id,
        &current_password,
        &new_password,
        controller_ciphertext,
        (!invitation_ciphertext.is_empty()).then_some(invitation_ciphertext),
    )
    .and_then(|vaults| encode(&vaults))
    .map_err(error)
}

#[wasm_bindgen(js_name = sealAccountRecovery)]
pub fn seal_account_recovery(
    scope: JsValue,
    password: Vec<u8>,
    backup: &[u8],
) -> std::result::Result<JsValue, JsValue> {
    let password = Zeroizing::new(password);
    decode(scope)
        .and_then(|scope| crate::recovery::seal(&scope, &password, backup))
        .and_then(|backup| encode(&backup))
        .map_err(error)
}

#[wasm_bindgen(js_name = openAccountRecovery)]
pub fn open_account_recovery(
    scope: JsValue,
    password: Vec<u8>,
    ciphertext: &[u8],
) -> std::result::Result<JsValue, JsValue> {
    let password = Zeroizing::new(password);
    decode(scope)
        .and_then(|scope| crate::recovery::open(&scope, &password, ciphertext))
        .and_then(|backup| encode(&backup))
        .map_err(error)
}

#[wasm_bindgen(js_name = signTelemetryRoster)]
pub fn sign_telemetry_roster(
    roster: JsValue,
    password: Vec<u8>,
    ciphertext: &[u8],
) -> std::result::Result<String, JsValue> {
    let password = Zeroizing::new(password);
    decode(roster)
        .and_then(|roster| controller::approve_telemetry_roster(&roster, &password, ciphertext))
        .map_err(error)
}

#[wasm_bindgen(js_name = verifyDeviceReceipt)]
pub fn verify_device_receipt(
    receipt: JsValue,
    expected_manifest: &str,
    controller_key: JsValue,
) -> std::result::Result<JsValue, JsValue> {
    let receipt = decode(receipt).map_err(error)?;
    let key = decode(controller_key).map_err(error)?;
    controller::verify_device_receipt(&receipt, expected_manifest, &key)
        .and_then(|manifest| encode(&manifest))
        .map_err(error)
}

#[wasm_bindgen(js_name = verifyTelemetryRoster)]
pub fn verify_telemetry_roster(
    compact: &str,
    owner_key: JsValue,
    now: f64,
) -> std::result::Result<JsValue, JsValue> {
    let key = decode(owner_key).map_err(error)?;
    flow_like_device_protocol::verify_telemetry_roster(compact, &key, time(now).map_err(error)?)
        .map_err(error)
        .and_then(|roster| encode(&roster).map_err(error))
}

#[wasm_bindgen(js_name = signArchiveRoster)]
pub fn sign_archive_roster(
    roster: JsValue,
    password: Vec<u8>,
    ciphertext: &[u8],
) -> std::result::Result<String, JsValue> {
    let password = Zeroizing::new(password);
    decode(roster)
        .and_then(|roster| controller::approve_archive_roster(&roster, &password, ciphertext))
        .map_err(error)
}

#[wasm_bindgen(js_name = verifyHistoricalTelemetryRoster)]
pub fn verify_historical_telemetry_roster(
    compact: &str,
    owner_key: JsValue,
) -> std::result::Result<JsValue, JsValue> {
    let key = decode(owner_key).map_err(error)?;
    flow_like_device_protocol::verify_historical_telemetry_roster(compact, &key)
        .map_err(error)
        .and_then(|roster| encode(&roster).map_err(error))
}

#[wasm_bindgen(js_name = verifyArchiveRosterHead)]
pub fn verify_archive_roster_head(
    compact: &str,
    owner_key: JsValue,
) -> std::result::Result<JsValue, JsValue> {
    let key = decode(owner_key).map_err(error)?;
    flow_like_device_protocol::verify_archive_roster_head(compact, &key)
        .map_err(error)
        .and_then(|roster| encode(&roster).map_err(error))
}

#[wasm_bindgen(js_name = verifyManagementPolicy)]
pub fn verify_management_policy(
    compact: &str,
    owner_key: JsValue,
) -> std::result::Result<JsValue, JsValue> {
    let owner_key = decode(owner_key).map_err(error)?;
    flow_like_device_protocol::verify_historical_management_policy(compact, &owner_key)
        .map_err(error)
        .and_then(|policy| encode(&policy).map_err(error))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Audience {
    scope: String,
    owner_invitation_key: Ed25519PublicKey,
    publisher: TelemetryMember,
}

#[wasm_bindgen]
pub struct BrowserController {
    controller: Option<controller::UnlockedController>,
    registry: Rc<Registry>,
}

impl BrowserController {
    fn controller(&self) -> Result<&controller::UnlockedController> {
        self.controller.as_ref().context("Controller is locked")
    }
    fn pins(&self, audience: JsValue) -> Result<MlsPins> {
        let audience: Audience = decode(audience)?;
        let public = self.controller()?.public_bundle();
        Ok(MlsPins {
            device_id: public.device_id,
            scope: audience.scope,
            owner_invitation_key: audience.owner_invitation_key,
            publisher: MemberIdentity::new(
                audience.publisher.endpoint_id.into_bytes(),
                audience.publisher.signing_key.to_bytes()?,
            )?,
            local: MemberIdentity::new(
                public.endpoint_id.into_bytes(),
                public.telemetry_member.signing_key.to_bytes()?,
            )?,
        })
    }
}

#[wasm_bindgen]
impl BrowserController {
    #[wasm_bindgen(js_name = createFleetReader)]
    pub fn create_fleet_reader(
        &self,
        api_base_url: String,
        user_id: String,
        revision: u64,
        issued_at: i64,
        expires_at: i64,
    ) -> std::result::Result<String, JsValue> {
        self.controller()
            .and_then(|controller| {
                controller.create_fleet_reader(
                    api_base_url,
                    user_id,
                    revision,
                    issued_at,
                    expires_at,
                )
            })
            .map_err(error)
    }

    #[wasm_bindgen(js_name = verifyFleetReader)]
    pub fn verify_fleet_reader(
        &self,
        trusted: JsValue,
        receipt: JsValue,
        compact: &str,
    ) -> std::result::Result<JsValue, JsValue> {
        let trusted = decode(trusted).map_err(error)?;
        let receipt = decode(receipt).map_err(error)?;
        self.controller()
            .and_then(|controller| {
                encode(&controller.verify_fleet_reader_history(&trusted, &receipt, compact)?)
            })
            .map_err(error)
    }

    #[wasm_bindgen(js_name = verifyFleetView)]
    pub fn verify_fleet_view(
        &self,
        trusted: JsValue,
        receipt: JsValue,
        view: JsValue,
        now: i64,
    ) -> std::result::Result<JsValue, JsValue> {
        let trusted = decode(trusted).map_err(error)?;
        let receipt = decode(receipt).map_err(error)?;
        let view = decode(view).map_err(error)?;
        self.controller()
            .and_then(|controller| {
                encode(&controller.verify_fleet_view(&trusted, &receipt, &view, now)?)
            })
            .map_err(error)
    }

    #[wasm_bindgen(js_name = openFleet)]
    pub fn open_fleet(
        &self,
        trusted: JsValue,
        receipt: JsValue,
        view: JsValue,
        bundle: JsValue,
        now: i64,
    ) -> std::result::Result<Vec<u8>, JsValue> {
        let trusted = decode(trusted).map_err(error)?;
        let receipt = decode(receipt).map_err(error)?;
        let view = decode(view).map_err(error)?;
        let bundle = decode(bundle).map_err(error)?;
        self.controller()
            .and_then(|controller| controller.open_fleet(&trusted, &receipt, &view, &bundle, now))
            .map(|plaintext| plaintext.to_vec())
            .map_err(error)
    }

    #[wasm_bindgen(js_name = sealInventory)]
    pub fn seal_inventory(
        &self,
        binding: JsValue,
        plaintext: &[u8],
    ) -> std::result::Result<JsValue, JsValue> {
        let binding = decode(binding).map_err(error)?;
        self.controller()
            .and_then(|controller| encode(&controller.seal_inventory(binding, plaintext)?))
            .map_err(error)
    }

    #[wasm_bindgen(js_name = openInventory)]
    pub fn open_inventory(
        &self,
        expected: JsValue,
        encrypted: JsValue,
    ) -> std::result::Result<Vec<u8>, JsValue> {
        let expected = decode(expected).map_err(error)?;
        let encrypted = decode(encrypted).map_err(error)?;
        self.controller()
            .and_then(|controller| controller.open_inventory(&expected, &encrypted))
            .map(|plaintext| plaintext.to_vec())
            .map_err(error)
    }

    #[wasm_bindgen(js_name = openArchive)]
    pub fn open_archive(
        &self,
        pins: JsValue,
        bundle: JsValue,
        recipient_id: &str,
    ) -> std::result::Result<Vec<u8>, JsValue> {
        let pins = decode(pins).map_err(error)?;
        let bundle = decode(bundle).map_err(error)?;
        self.controller()
            .and_then(|controller| controller.open_archive(&pins, &bundle, recipient_id))
            .map(|plaintext| plaintext.to_vec())
            .map_err(error)
    }
    #[wasm_bindgen(js_name = publicBundle)]
    pub fn public_bundle(&self) -> std::result::Result<JsValue, JsValue> {
        self.controller()
            .and_then(|controller| encode(&controller.public_bundle()))
            .map_err(error)
    }

    #[wasm_bindgen(js_name = freshEndpointVault)]
    pub fn fresh_endpoint_vault(
        &mut self,
        password: Vec<u8>,
    ) -> std::result::Result<JsValue, JsValue> {
        let password = Zeroizing::new(password);
        let result = self
            .controller
            .as_mut()
            .context("Controller is locked")
            .and_then(|controller| controller.fresh_endpoint_vault(&password))
            .map_err(error)?;
        self.registry.close();
        self.registry = Rc::new(Registry::default());
        encode(&result).map_err(error)
    }

    #[wasm_bindgen(js_name = completeOnboarding)]
    pub fn complete_onboarding(
        &mut self,
        manifest: JsValue,
        password: Vec<u8>,
        invitation_vault: &[u8],
    ) -> std::result::Result<JsValue, JsValue> {
        let password = Zeroizing::new(password);
        let manifest = decode(manifest).map_err(error)?;
        self.controller
            .as_mut()
            .context("Controller is locked")
            .and_then(|controller| {
                controller.complete_onboarding(&manifest, &password, invitation_vault)
            })
            .and_then(|result| encode(&result))
            .map_err(error)
    }

    #[wasm_bindgen(js_name = signOnboardingManifest)]
    pub fn sign_onboarding_manifest(
        &self,
        manifest: JsValue,
    ) -> std::result::Result<String, JsValue> {
        let manifest = decode(manifest).map_err(error)?;
        self.controller()
            .and_then(|controller| controller.sign_onboarding_manifest(&manifest))
            .map_err(error)
    }

    #[wasm_bindgen(js_name = beginNoise)]
    pub fn begin_noise(
        &self,
        grant_id: &str,
        device_key: &[u8],
        now: f64,
    ) -> std::result::Result<BrowserNoiseHandshake, JsValue> {
        let device_key: [u8; 32] = device_key
            .try_into()
            .map_err(|_| error("Invalid device Noise key size"))?;
        let certified = self
            .controller()
            .and_then(|controller| controller.begin_noise(grant_id, device_key, time(now)?))
            .map_err(error)?;
        let expires_at = certified.certificate.expires_at;
        let session_id = certified.certificate.session_id;
        let certificate = certified.certificate_jws;
        Ok(BrowserNoiseHandshake {
            inner: self.registry.register(certified.handshake).map_err(error)?,
            expires_at,
            session_id,
            certificate,
        })
    }

    #[wasm_bindgen(js_name = createTelemetry)]
    pub fn create_telemetry(
        &self,
        audience: JsValue,
    ) -> std::result::Result<BrowserMlsEndpoint, JsValue> {
        let pins = self.pins(audience).map_err(error)?;
        let controller = self.controller().map_err(error)?;
        let endpoint = PreparedMlsEndpoint::create(
            pins,
            controller.storage_key(),
            &controller.telemetry_key(),
        )
        .map_err(error)?;
        Ok(BrowserMlsEndpoint {
            inner: self.registry.register(endpoint).map_err(error)?,
        })
    }

    #[wasm_bindgen(js_name = openTelemetry)]
    pub fn open_telemetry(
        &self,
        audience: JsValue,
        snapshot: JsValue,
        checkpoint: JsValue,
    ) -> std::result::Result<BrowserMlsEndpoint, JsValue> {
        let pins = self.pins(audience).map_err(error)?;
        let snapshot = decode(snapshot).map_err(error)?;
        let checkpoint = decode(checkpoint).map_err(error)?;
        let endpoint = PreparedMlsEndpoint::open(
            pins,
            self.controller().map_err(error)?.storage_key(),
            snapshot,
            checkpoint,
        )
        .map_err(error)?;
        Ok(BrowserMlsEndpoint {
            inner: self.registry.register(endpoint).map_err(error)?,
        })
    }

    /// Close clears this unlock and every derived Noise/MLS session, including
    /// prepared plaintext that has not yet been released after persistence.
    pub fn close(&mut self) {
        self.registry.close();
        self.controller = None;
    }
}
impl Drop for BrowserController {
    fn drop(&mut self) {
        self.close();
    }
}

#[wasm_bindgen]
pub struct BrowserNoiseHandshake {
    inner: Handle<noise::Handshake>,
    expires_at: i64,
    session_id: String,
    certificate: String,
}
#[wasm_bindgen]
impl BrowserNoiseHandshake {
    pub fn certificate(&self) -> String {
        self.certificate.clone()
    }
    #[wasm_bindgen(js_name = sessionId)]
    pub fn session_id(&self) -> String {
        self.session_id.clone()
    }
    pub fn write(&self, now: f64) -> std::result::Result<Vec<u8>, JsValue> {
        self.inner
            .with(|handshake| {
                ensure!(
                    time(now)? < self.expires_at,
                    "Noise session certificate expired"
                );
                Ok(handshake.write()?)
            })
            .map_err(error)
    }
    pub fn read(&self, message: &[u8], now: f64) -> std::result::Result<(), JsValue> {
        self.inner
            .with(|handshake| {
                ensure!(
                    time(now)? < self.expires_at,
                    "Noise session certificate expired"
                );
                Ok(handshake.read(message)?)
            })
            .map_err(error)
    }
    pub fn finish(self, now: f64) -> std::result::Result<BrowserNoiseSession, JsValue> {
        ensure_session_time(now, self.expires_at).map_err(error)?;
        let session = self.inner.take().map_err(error)?.finish().map_err(error)?;
        Ok(BrowserNoiseSession {
            inner: self.inner.registry.register(session).map_err(error)?,
            expires_at: self.expires_at,
        })
    }
    pub fn close(&self) {
        self.inner.close();
    }
}
fn ensure_session_time(now: f64, expires_at: i64) -> Result<()> {
    ensure!(time(now)? < expires_at, "Noise session certificate expired");
    Ok(())
}

#[wasm_bindgen]
pub struct BrowserNoiseSession {
    inner: Handle<noise::Session>,
    expires_at: i64,
}
#[wasm_bindgen]
impl BrowserNoiseSession {
    pub fn encrypt(&self, plaintext: &[u8], now: f64) -> std::result::Result<Vec<u8>, JsValue> {
        self.inner
            .with(|session| {
                ensure_session_time(now, self.expires_at)?;
                Ok(session.encrypt(plaintext)?)
            })
            .map_err(error)
    }
    pub fn decrypt(&self, ciphertext: &[u8], now: f64) -> std::result::Result<Vec<u8>, JsValue> {
        self.inner
            .with(|session| {
                ensure_session_time(now, self.expires_at)?;
                Ok(session.decrypt(ciphertext)?)
            })
            .map_err(error)
    }
    pub fn close(&self) {
        self.inner.close();
    }
}

#[wasm_bindgen]
pub struct BrowserMlsEndpoint {
    inner: Handle<PreparedMlsEndpoint>,
}
#[wasm_bindgen]
impl BrowserMlsEndpoint {
    #[wasm_bindgen(js_name = deliveryReceipts)]
    pub fn delivery_receipts(&self) -> std::result::Result<JsValue, JsValue> {
        self.inner
            .with(|endpoint| encode(&endpoint.delivery_receipts()?))
            .map_err(error)
    }
    #[wasm_bindgen(js_name = prepareReceiptConfirmation)]
    pub fn prepare_receipt_confirmation(
        &self,
        sequence: f64,
        envelope_digest: &str,
    ) -> std::result::Result<(), JsValue> {
        let sequence = time(sequence).map_err(error)? as u64;
        self.inner
            .with(|endpoint| endpoint.prepare_receipt_confirmation(sequence, envelope_digest))
            .map_err(error)
    }
    pub fn position(&self) -> std::result::Result<JsValue, JsValue> {
        self.inner
            .with(|endpoint| {
                let (joined, retired, sequence) = endpoint.reader_position()?;
                encode(&serde_json::json!({"joined":joined,"retired":retired,"sequence":sequence}))
            })
            .map_err(error)
    }
    #[wasm_bindgen(js_name = preparedSnapshot)]
    pub fn prepared_snapshot(&self) -> std::result::Result<JsValue, JsValue> {
        self.inner
            .with(|endpoint| encode(&endpoint.prepared()?))
            .map_err(error)
    }
    pub fn checkpoint(&self) -> std::result::Result<JsValue, JsValue> {
        self.inner
            .with(|endpoint| encode(&endpoint.checkpoint()))
            .map_err(error)
    }
    #[wasm_bindgen(js_name = confirmCommit)]
    pub fn confirm_commit(&self, checkpoint: JsValue) -> std::result::Result<JsValue, JsValue> {
        let checkpoint = decode(checkpoint).map_err(error)?;
        self.inner
            .with(|endpoint| {
                let output = endpoint.confirm_commit(&checkpoint)?;
                let json = std::str::from_utf8(&output).context("Invalid prepared result")?;
                js_sys::JSON::parse(json).map_err(|_| anyhow::anyhow!("Invalid prepared result"))
            })
            .map_err(error)
    }
    #[wasm_bindgen(js_name = discardPrepared)]
    pub fn discard_prepared(&self) -> std::result::Result<(), JsValue> {
        self.inner
            .with(|endpoint| {
                endpoint.discard_prepared();
                Ok(())
            })
            .map_err(error)
    }
    #[wasm_bindgen(js_name = prepareKeyPackage)]
    pub fn prepare_key_package(&self) -> std::result::Result<(), JsValue> {
        self.inner
            .with(|endpoint| endpoint.prepare_key_package())
            .map_err(error)
    }
    #[wasm_bindgen(js_name = prepareJoin)]
    pub fn prepare_join(&self, delivery: JsValue, now: f64) -> std::result::Result<(), JsValue> {
        let delivery = decode(delivery).map_err(error)?;
        self.inner
            .with(|endpoint| endpoint.prepare_join(&delivery, time(now)?))
            .map_err(error)
    }
    #[wasm_bindgen(js_name = prepareReceive)]
    pub fn prepare_receive(&self, delivery: JsValue, now: f64) -> std::result::Result<(), JsValue> {
        let delivery = decode(delivery).map_err(error)?;
        self.inner
            .with(|endpoint| endpoint.prepare_receive(&delivery, time(now)?))
            .map_err(error)
    }
    pub fn close(&self) {
        self.inner.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locking_controller_drops_all_child_secrets_and_pending_results() {
        struct Secret(Rc<Cell<bool>>);
        impl Drop for Secret {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let registry = Rc::new(Registry::default());
        let dropped = Rc::new(Cell::new(false));
        let session = registry.register(Secret(dropped.clone())).unwrap();
        registry.close();
        assert!(dropped.get());
        assert!(session.with(|_| Ok(())).is_err());
        assert!(registry.register(0).is_err());
    }
}
