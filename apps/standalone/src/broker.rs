use crate::{
    config::PlacementConfig,
    enrollment::{DeviceSession, api_status, http_client, response_json, unix_time},
    state::StateStore,
};
use anyhow::{Context, Result, ensure};
#[cfg(feature = "runtime")]
use flow_like_device_protocol::InstanceStorageLease;
use flow_like_device_protocol::{
    ClientAssertion, DpopProof, InstancePossession, InstancePurpose, InstanceReceipt,
    InstanceRegistration, InstanceRegistrationRequest, InstanceTokenRequest, InstanceTokenResponse,
    PROTOCOL_VERSION, ReceiptRequest, SigningKey, access_token_hash, compact_digest, endpoint_url,
    sign_dpop, sign_instance_possession, sign_workload_assertion,
};
use flow_like_types_contracts::authorization::{
    AuthorizationError, AuthorizationFuture, AuthorizationRequest, RequestAuthorization,
    RequestAuthorizer, ResourceAudience,
};
use rand_core::{OsRng, RngCore};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

struct ResourceLease {
    token: Zeroizing<String>,
    nonce: String,
    expires_at: i64,
    refresh_at: i64,
}

#[derive(Default)]
struct BrokerState {
    receipt: Option<InstanceReceipt>,
    registrations: Vec<String>,
    lease: Option<ResourceLease>,
    denied: bool,
    project_lease: Option<ResourceLease>,
    project_denied: bool,
    #[cfg(feature = "runtime")]
    outage_restored_until: Option<i64>,
    #[cfg(feature = "runtime")]
    ready: bool,
}

/// One ephemeral workload key and lease per supervised process. Only public
/// lifecycle metadata reaches SQLite; a restarted process gets a new identity.
pub struct WorkloadBroker {
    device: Arc<DeviceSession>,
    config: PlacementConfig,
    state_dir: PathBuf,
    instance_id: String,
    key: SigningKey,
    client: reqwest::Client,
    state: Mutex<BrokerState>,
    validation: Option<String>,
}

enum LaunchBinding {
    Workload {
        config_revision: u64,
        intent_revision: u64,
        replica_slot: u8,
    },
    Validation {
        rollout_id: String,
    },
}

impl WorkloadBroker {
    pub fn new(
        device: Arc<DeviceSession>,
        config: PlacementConfig,
        state_dir: PathBuf,
        config_revision: u64,
        intent_revision: u64,
    ) -> Result<Self> {
        Self::new_replica(
            device,
            config,
            state_dir,
            config_revision,
            intent_revision,
            0,
        )
    }

    pub(crate) fn new_replica(
        device: Arc<DeviceSession>,
        config: PlacementConfig,
        state_dir: PathBuf,
        config_revision: u64,
        intent_revision: u64,
        replica_slot: u8,
    ) -> Result<Self> {
        Self::new_bound(
            device,
            config,
            state_dir,
            LaunchBinding::Workload {
                config_revision,
                intent_revision,
                replica_slot,
            },
        )
    }

    pub(crate) fn new_rollout_validation(
        device: Arc<DeviceSession>,
        config: PlacementConfig,
        state_dir: PathBuf,
        rollout_id: String,
    ) -> Result<Self> {
        Self::new_bound(
            device,
            config,
            state_dir,
            LaunchBinding::Validation { rollout_id },
        )
    }

    fn new_bound(
        device: Arc<DeviceSession>,
        config: PlacementConfig,
        state_dir: PathBuf,
        launch: LaunchBinding,
    ) -> Result<Self> {
        config.validate()?;
        ensure!(
            config.resource_grant.is_some(),
            "Placement has no resource authorization"
        );
        let instance_id = Uuid::new_v4().to_string();
        let key = SigningKey::generate();
        let client = http_client()?;
        let mut binding = serde_json::json!({"instance_id":instance_id,"placement_id":config.id,"project_id":config.project_id,
            "deployment_id":config.deployment_id,"resource_grant":config.resource_grant,
            "workload_key":key.public_key()});
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let validation = store.with_rollout_transaction(|| {
            let validation = match launch {
                LaunchBinding::Workload {
                    config_revision,
                    intent_revision,
                    replica_slot,
                } => {
                    binding["config_revision"] = config_revision.into();
                    binding["intent_revision"] = intent_revision.into();
                    binding["replica_slot"] = replica_slot.into();
                    binding["purpose"] = serde_json::to_value(InstancePurpose::Workload)?;
                    None
                }
                LaunchBinding::Validation { rollout_id } => {
                    ensure!(
                        config.source == crate::config::ProjectSource::Online,
                        "Validation credentials require an online placement"
                    );
                    let rollout =
                        store.require_rollout_validation(&rollout_id, &config, unix_time()?)?;
                    binding["config_revision"] = rollout.base_revision.into();
                    binding["intent_revision"] = rollout.base_intent.into();
                    binding["rollout_id"] = rollout_id.clone().into();
                    binding["purpose"] = serde_json::to_value(InstancePurpose::RolloutValidation)?;
                    Some(rollout_id)
                }
            };
            // No cloud admission is possible until reserve_possible_lease runs
            // immediately before a request. Cache-only restarts reserve no slot.
            store.begin_instance(&instance_id, &config.id, &binding, unix_time()?)?;
            Ok(validation)
        })?;
        Ok(Self {
            device,
            config,
            state_dir,
            instance_id,
            key,
            client,
            state: Mutex::new(BrokerState::default()),
            validation,
        })
    }

    fn purpose(&self) -> InstancePurpose {
        if self.validation.is_some() {
            InstancePurpose::RolloutValidation
        } else {
            InstancePurpose::Workload
        }
    }

    fn require_current_validation(&self) -> Result<()> {
        if let Some(id) = &self.validation {
            let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
            store
                .require_rollout_validation(id, &self.config, unix_time()?)
                .map_err(|_| AuthorizationError::Denied)?;
            let active: bool = store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM workload_instances WHERE instance_id=?1 AND retire_pending=0)",
                [&self.instance_id], |row| row.get(0),
            )?;
            if !active {
                return Err(AuthorizationError::Denied.into());
            }
        }
        Ok(())
    }

    pub(crate) async fn retire_validation(&self) -> Result<()> {
        ensure!(self.validation.is_some(), "Expected a validation instance");
        {
            let mut state = self.state.lock().await;
            state.project_denied = true;
            state.denied = true;
            state.project_lease = None;
            state.lease = None;
        }
        let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
        store.retire_instance(&self.instance_id)?;
        drop(store);
        // Release validation capacity before the next preflight. A lost response
        // remains in the durable retirement queue for the background drainer.
        if tokio::time::timeout(
            Duration::from_secs(5),
            self.device.retire_instance(&self.instance_id),
        )
        .await
        .is_ok_and(|result| matches!(result, Ok(true)))
        {
            StateStore::open(&self.state_dir.join("management.sqlite"))?
                .forget_instance(&self.instance_id)?;
        }
        Ok(())
    }

    pub(crate) async fn prepare(&self) -> Result<()> {
        self.require_current_validation()?;
        let mut state = self.state.lock().await;
        let result = self.ensure_registered(&mut state).await;
        if result.as_ref().err().is_some_and(|error| {
            matches!(
                api_status(error),
                Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
            )
        }) {
            state.project_denied = true;
            #[cfg(feature = "runtime")]
            self.fence_outage()?;
        }
        #[cfg(feature = "runtime")]
        if let Err(error) = &result {
            if crate::online::authorization_error(error) == AuthorizationError::Unavailable
                && state
                    .outage_restored_until
                    .is_some_and(|expiry| expiry > unix_time().unwrap_or(i64::MAX))
            {
                state.ready = true;
                return self.require_current_validation();
            }
        }
        result?;
        #[cfg(feature = "runtime")]
        {
            state.ready = true;
        }
        self.require_current_validation()
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn identity(&self) -> crate::config::WorkloadIdentity {
        crate::config::WorkloadIdentity {
            instance_id: self.instance_id.clone(),
            device_id: self.device.manifest().device_id.clone(),
            device_auth_epoch: self.device.auth_epoch(),
            key_epoch: 1,
        }
    }

    fn base(&self) -> &str {
        &self.device.manifest().api_base_url
    }

    fn workload_assertion(&self, endpoint: &str) -> Result<String> {
        let now = unix_time()?;
        Ok(sign_workload_assertion(
            &ClientAssertion {
                iss: self.instance_id.clone(),
                sub: self.instance_id.clone(),
                aud: endpoint.into(),
                iat: now,
                nbf: now,
                exp: now + 60,
                jti: Uuid::new_v4().to_string(),
            },
            &self.key,
        )?)
    }

    fn accept_receipt(&self, state: &mut BrokerState, receipt: InstanceReceipt) -> Result<()> {
        let grant = self
            .config
            .resource_grant
            .as_ref()
            .context("Missing resource grant")?;
        ensure!(
            receipt.instance_id == self.instance_id
                && receipt.device_id == self.device.manifest().device_id
                && receipt.grant_id == grant.grant_id
                && receipt.billing_grant_id
                    == if self.validation.is_some() {
                        None
                    } else {
                        grant.billing_grant_id.clone()
                    }
                && receipt.purpose == self.purpose()
                && receipt.workload_key == self.key.public_key()
                && receipt.key_epoch == 1
                && state.registrations.contains(&receipt.registration_jws),
            "Instance receipt changed its authorized identity"
        );
        ensure!(
            receipt.registered_at > 0
                && receipt.registered_at <= unix_time()? + 5
                && receipt.lease_expires_at > receipt.registered_at
                && receipt.lease_expires_at <= unix_time()? + 605,
            "Invalid instance registration lease"
        );
        StateStore::open(&self.state_dir.join("management.sqlite"))?
            .update_instance_lease(&self.instance_id, receipt.lease_expires_at)?;
        state.receipt = Some(receipt);
        Ok(())
    }

    async fn ensure_registered(&self, state: &mut BrokerState) -> Result<()> {
        if state.receipt.is_some() {
            return Ok(());
        }
        if self.validation.is_none()
            && StateStore::open(&self.state_dir.join("management.sqlite"))?
                .has_pending_serving_retirements(&self.config.id, unix_time()?)?
        {
            // A restored worker may read its validated cache immediately. Cloud
            // admission still waits for the previous serving lease to retire.
            return Err(AuthorizationError::Unavailable.into());
        }
        if !state.registrations.is_empty() {
            let endpoint = endpoint_url(
                self.base(),
                &format!("/instances/{}/receipt", self.instance_id),
            )?;
            let receipt = response_json::<InstanceReceipt>(
                self.client
                    .post(&endpoint)
                    .json(&ReceiptRequest {
                        client_assertion: self.workload_assertion(&endpoint)?,
                    })
                    .send()
                    .await?,
            )
            .await;
            match receipt {
                Ok(receipt) => return self.accept_receipt(state, receipt),
                Err(error) if api_status(&error) == Some(reqwest::StatusCode::NOT_FOUND) => {}
                Err(error) => return Err(error),
            }
        }
        // Receipt recovery runs before another admission. Bound retained proofs,
        // without permanently exhausting a process during an extended outage.
        if state.registrations.len() == 16 {
            state.registrations.remove(0);
        }
        let grant = self
            .config
            .resource_grant
            .as_ref()
            .context("Missing resource grant")?;
        let endpoint = endpoint_url(
            self.base(),
            &format!("/devices/{}/instances", self.device.manifest().device_id),
        )?;
        let now = unix_time()?;
        let registration_jws = self
            .device
            .sign_instance_registration(InstanceRegistration {
                version: PROTOCOL_VERSION,
                purpose: self.purpose(),
                device_id: self.device.manifest().device_id.clone(),
                device_auth_epoch: self.device.auth_epoch(),
                instance_id: self.instance_id.clone(),
                placement_id: self.config.id.clone(),
                deployment_id: self.config.deployment_id.clone(),
                project_id: self.config.project_id.clone(),
                grant_id: grant.grant_id.clone(),
                authz_version: grant.authz_version,
                billing_grant_id: if self.validation.is_some() {
                    None
                } else {
                    grant.billing_grant_id.clone()
                },
                billing_authz_version: if self.validation.is_some() {
                    None
                } else {
                    grant.billing_authz_version
                },
                workload_key: self.key.public_key(),
                aud: endpoint.clone(),
                iat: now,
                nbf: now,
                exp: now + 60,
                jti: Uuid::new_v4().to_string(),
            })?;
        let possession_jws = sign_instance_possession(
            &InstancePossession {
                iss: self.instance_id.clone(),
                sub: self.instance_id.clone(),
                aud: endpoint.clone(),
                iat: now,
                nbf: now,
                exp: now + 60,
                jti: Uuid::new_v4().to_string(),
                registration_digest: compact_digest(&registration_jws),
            },
            &self.key,
        )?;
        state.registrations.push(registration_jws.clone());
        self.reserve_possible_lease()?;
        let receipt = response_json(
            self.client
                .post(&endpoint)
                .json(&InstanceRegistrationRequest {
                    registration_jws,
                    possession_jws,
                })
                .send()
                .await?,
        )
        .await?;
        self.accept_receipt(state, receipt)
    }

    async fn authorize_inner(
        &self,
        audience: ResourceAudience,
        method: &str,
        url: &str,
    ) -> Result<RequestAuthorization> {
        let mut state = self.state.lock().await;
        self.require_current_validation()?;
        let project = audience == ResourceAudience::ProjectApi;
        if if project {
            state.project_denied
        } else {
            state.denied
        } {
            return Err(AuthorizationError::Denied.into());
        }
        if let Err(error) = self.ensure_registered(&mut state).await {
            if matches!(
                api_status(&error),
                Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
            ) {
                state.project_denied = true;
                #[cfg(feature = "runtime")]
                self.fence_outage()?;
            }
            return Err(error);
        }
        let now = unix_time()?;
        if (if project {
            &state.project_lease
        } else {
            &state.lease
        })
        .as_ref()
        .is_none_or(|lease| now >= lease.refresh_at)
        {
            if project {
                state.project_lease = None;
            } else {
                state.lease = None;
            }
            let endpoint = endpoint_url(
                self.base(),
                &format!(
                    "/instances/{}/{}",
                    self.instance_id,
                    if project { "project-token" } else { "token" }
                ),
            )?;
            self.reserve_possible_lease()?;
            let response = response_json::<InstanceTokenResponse>(
                self.client
                    .post(&endpoint)
                    .json(&InstanceTokenRequest {
                        client_assertion: self.workload_assertion(&endpoint)?,
                    })
                    .send()
                    .await?,
            )
            .await;
            let mut response = match response {
                Ok(response) => response,
                Err(error) => {
                    if matches!(
                        api_status(&error),
                        Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
                    ) {
                        if project {
                            state.project_denied = true;
                            #[cfg(feature = "runtime")]
                            self.fence_outage()?;
                        } else {
                            state.denied = true;
                        }
                    }
                    return Err(error);
                }
            };
            let now = unix_time()?;
            ensure!(
                response.token_type == "DPoP"
                    && response.expires_in > 60
                    && response.expires_in <= 300
                    && response.expires_at > now + 60
                    && response.expires_at <= now + 305
                    && response.lease_expires_at >= response.expires_at
                    && response.lease_expires_at <= now + 605
                    && (16..=256).contains(&response.dpop_nonce.len())
                    && response
                        .dpop_nonce
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic()),
                "Invalid instance resource lease"
            );
            StateStore::open(&self.state_dir.join("management.sqlite"))?
                .update_instance_lease(&self.instance_id, response.lease_expires_at)?;
            let lease = Some(ResourceLease {
                token: Zeroizing::new(std::mem::take(&mut response.access_token)),
                nonce: response.dpop_nonce,
                expires_at: response.expires_at,
                refresh_at: response.expires_at - 60 - i64::from(OsRng.next_u32() % 16),
            });
            if project {
                state.project_lease = lease;
            } else {
                state.lease = lease;
            }
        }
        let lease = (if project {
            &state.project_lease
        } else {
            &state.lease
        })
        .as_ref()
        .context("Missing resource lease")?;
        let proof = sign_dpop(
            &DpopProof {
                jti: Uuid::new_v4().to_string(),
                htm: method.into(),
                htu: url.into(),
                iat: unix_time()?,
                ath: Some(access_token_hash(&lease.token)),
                nonce: Some(lease.nonce.clone()),
            },
            &self.key,
        )?;
        self.require_current_validation()?;
        Ok(RequestAuthorization::new(
            format!("DPoP {}", &*lease.token),
            Some(proof),
            UNIX_EPOCH + Duration::from_secs(lease.expires_at.try_into()?),
        )?)
    }

    fn reserve_possible_lease(&self) -> Result<()> {
        // A response can be lost after admission. Keep retirement metadata past
        // the latest lease the server can mint from this 60-second assertion.
        let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
        store.with_rollout_transaction(|| {
            if let Some(id) = &self.validation {
                store
                    .require_rollout_validation(id, &self.config, unix_time()?)
                    .map_err(|_| AuthorizationError::Denied)?;
            }
            store.update_instance_lease(&self.instance_id, unix_time()? + 660)
        })
    }
}

impl Drop for WorkloadBroker {
    fn drop(&mut self) {
        if self.validation.is_some() {
            // Also runs when an in-flight validation is cancelled or times out.
            if let Ok(store) = StateStore::open(&self.state_dir.join("management.sqlite")) {
                let _ = store.retire_instance(&self.instance_id);
            }
        }
    }
}

#[cfg(feature = "runtime")]
fn checkpoint_mac(
    device: &DeviceSession,
    claim: &crate::online::outage::SnapshotClaim,
) -> Result<hmac::Hmac<sha2::Sha256>> {
    use hmac::Mac;
    let key = zeroize::Zeroizing::new(device.telemetry_signer().to_bytes());
    let mut mac = <hmac::Hmac<sha2::Sha256> as hmac::Mac>::new_from_slice(key.as_ref())?;
    mac.update(b"flow-like/online-outage-snapshot/v1\0");
    mac.update(&serde_json::to_vec(claim)?);
    Ok(mac)
}

#[cfg(feature = "runtime")]
impl WorkloadBroker {
    pub(crate) fn can_restore_outage(
        device: &DeviceSession,
        config: &PlacementConfig,
        state_dir: &std::path::Path,
    ) -> Result<bool> {
        if config.source != crate::config::ProjectSource::Online {
            return Ok(false);
        }
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let exists: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='online_outage_authority')", [], |row| row.get(0))?;
        if !exists {
            return Ok(false);
        }
        let identity = crate::config::WorkloadIdentity {
            instance_id: "outage-probe".into(),
            device_id: device.manifest().device_id.clone(),
            device_auth_epoch: device.auth_epoch(),
            key_epoch: 1,
        };
        let binding = crate::online::outage::binding(
            config,
            &identity,
            &format!("{}/instances/project", device.manifest().api_base_url),
        )?;
        let allowed: bool = store.connection.query_row("SELECT EXISTS(SELECT 1 FROM online_outage_authority WHERE binding=?1 AND denied=0 AND expires_at>?2)", rusqlite::params![binding, unix_time()?], |row| row.get(0))?;
        if !allowed {
            return Ok(false);
        }
        let root = state_dir
            .join("placement-data")
            .join(&config.id)
            .join("current/store");
        let path = root
            .join(".standalone-cache")
            .join(&config.id)
            .join("outage/snapshot.json");
        if !path.try_exists()? {
            return Ok(false);
        }
        crate::placement_data::validate_root(&root, config)?;
        // Validate every mutable-cache parent before opening the private envelope.
        let _ = super::online::validate_snapshot_parent(&root, &config.id)?;
        let (claim, seal) = crate::online::outage::inspect_claim(&path, &binding)?;
        use base64::Engine;
        use hmac::Mac;
        if seal.len() != 43 {
            return Ok(false);
        }
        if checkpoint_mac(device, &claim)?
            .verify_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(seal)?)
            .is_err()
        {
            return Ok(false);
        }
        let deadline: i64 = store.connection.query_row(
            "SELECT expires_at FROM online_outage_authority WHERE binding=?1",
            [binding],
            |row| row.get(0),
        )?;
        Ok(claim.grant_expires_at == deadline && deadline > unix_time()?)
    }

    fn outage_binding(&self) -> Result<String> {
        crate::online::outage::binding(
            &self.config,
            &self.identity(),
            &format!("{}/instances/project", self.base()),
        )
    }

    fn outage_store(&self) -> Result<StateStore> {
        let store = StateStore::open(&self.state_dir.join("management.sqlite"))?;
        store.connection.execute_batch("CREATE TABLE IF NOT EXISTS online_outage_authority (binding TEXT PRIMARY KEY, expires_at INTEGER NOT NULL, denied INTEGER NOT NULL DEFAULT 0)")?;
        Ok(store)
    }

    fn fence_outage(&self) -> Result<()> {
        self.outage_store()?.connection.execute(
            "INSERT INTO online_outage_authority(binding,expires_at,denied) VALUES (?1,0,1) ON CONFLICT(binding) DO UPDATE SET denied=1",
            [self.outage_binding()?],
        )?;
        if let Err(error) = crate::online::revoke_initialized_cache(&self.state_dir, &self.config) {
            tracing::warn!(
                "Authorization is fenced, but local outage cache cleanup failed: {error}"
            );
        }
        Ok(())
    }

    fn outage_mac(
        &self,
        claim: &crate::online::outage::SnapshotClaim,
    ) -> Result<hmac::Hmac<sha2::Sha256>> {
        checkpoint_mac(&self.device, claim)
    }

    fn validate_outage_claim(&self, claim: &crate::online::outage::SnapshotClaim) -> Result<()> {
        claim.validate()?;
        ensure!(
            self.validation.is_none()
                && self.config.source == crate::config::ProjectSource::Online
                && claim.binding == self.outage_binding()?,
            "Outage snapshot is outside this workload configuration"
        );
        Ok(())
    }

    async fn authenticate_outage_context(
        &self,
        claim: &crate::online::outage::SnapshotClaim,
    ) -> Result<()> {
        let endpoint = endpoint_url(self.base(), "/instances/project/storage")?;
        let authorization = self
            .authorize_inner(ResourceAudience::ProjectApi, "POST", &endpoint)
            .await?;
        let mut token = reqwest::header::HeaderValue::from_str(authorization.authorization())?;
        token.set_sensitive(true);
        let mut proof = reqwest::header::HeaderValue::from_str(
            authorization.dpop().context("Missing workload proof")?,
        )?;
        proof.set_sensitive(true);
        // Authenticate the context independently of child memory. These provider
        // credentials exist only for this response and are never persisted.
        let response = response_json::<InstanceStorageLease>(
            self.client
                .post(&endpoint)
                .header("authorization", token)
                .header("dpop", proof)
                .send()
                .await?,
        )
        .await;
        let lease = match response {
            Ok(lease) => lease,
            Err(error) => {
                if matches!(
                    api_status(&error),
                    Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
                ) {
                    self.fence_outage()?;
                    self.state.lock().await.project_denied = true;
                    return Err(AuthorizationError::Denied.into());
                }
                return Err(error);
            }
        };
        let digest = crate::online::outage::authenticated_context_digest(
            &self.config,
            &self.identity(),
            &format!("{}/instances/project", self.base()),
            &lease,
        )?;
        ensure!(
            lease.grant_expires_at == Some(claim.grant_expires_at)
                && digest == claim.storage_context_digest,
            "Outage context differs from the authenticated project grant"
        );
        Ok(())
    }
}

#[cfg(feature = "runtime")]
#[async_trait::async_trait]
impl crate::online::outage::OutageAuthority for WorkloadBroker {
    async fn seal(&self, claim: &crate::online::outage::SnapshotClaim) -> Result<String> {
        use base64::Engine;
        use hmac::Mac;
        self.validate_outage_claim(claim)?;
        ensure!(
            claim.grant_expires_at > unix_time()?,
            AuthorizationError::Denied
        );
        {
            let state = self.state.lock().await;
            ensure!(
                !state.ready
                    && !state.project_denied
                    && state
                        .project_lease
                        .as_ref()
                        .is_some_and(|lease| lease.expires_at > unix_time().unwrap_or(i64::MAX)),
                "Outage snapshots can only be sealed during freshly authorized initialization"
            );
        }
        self.authenticate_outage_context(claim).await?;
        let state = self.state.lock().await;
        ensure!(
            !state.ready && !state.project_denied,
            "Outage initialization authority changed during the grant check"
        );
        let store = self.outage_store()?;
        store.connection.execute(
            "INSERT INTO online_outage_authority(binding,expires_at,denied) VALUES (?1,?2,0) ON CONFLICT(binding) DO UPDATE SET expires_at=excluded.expires_at WHERE denied=0",
            rusqlite::params![claim.binding, claim.grant_expires_at],
        )?;
        let (deadline, denied): (i64, bool) = store.connection.query_row(
            "SELECT expires_at,denied FROM online_outage_authority WHERE binding=?1",
            [&claim.binding],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            !denied && deadline == claim.grant_expires_at,
            AuthorizationError::Denied
        );
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(self.outage_mac(claim)?.finalize().into_bytes()))
    }

    async fn verify(&self, claim: &crate::online::outage::SnapshotClaim, seal: &str) -> Result<()> {
        use base64::Engine;
        use hmac::Mac;
        self.validate_outage_claim(claim)?;
        ensure!(seal.len() == 43, "Invalid outage authentication tag");
        self.outage_mac(claim)?
            .verify_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(seal)?)
            .map_err(|_| anyhow::anyhow!("Outage snapshot integrity verification failed"))?;
        let mut state = self.state.lock().await;
        ensure!(!state.project_denied, AuthorizationError::Denied);
        let (deadline, denied): (i64, bool) = self.outage_store()?.connection.query_row(
            "SELECT expires_at,denied FROM online_outage_authority WHERE binding=?1",
            [&claim.binding],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            !denied && deadline > unix_time()?,
            AuthorizationError::Denied
        );
        ensure!(
            deadline == claim.grant_expires_at,
            "A newer authenticated outage context replaced this snapshot"
        );
        state.outage_restored_until = Some(deadline);
        Ok(())
    }

    async fn deny(&self, binding: &str) -> Result<()> {
        ensure!(
            binding == self.outage_binding()?,
            "Cannot revoke another outage snapshot"
        );
        self.fence_outage()?;
        let mut state = self.state.lock().await;
        state.project_denied = true;
        state.project_lease = None;
        state.outage_restored_until = None;
        Ok(())
    }
}

pub(crate) fn allowed_model_request(base: &str, method: &str, url: &str) -> bool {
    method == "POST"
        && [
            "/instances/chat/completions",
            "/instances/responses",
            "/instances/embeddings/embed",
        ]
        .iter()
        .any(|path| endpoint_url(base, path).is_ok_and(|target| target == url))
}

pub(crate) fn allowed_project_request(base: &str, method: &str, url: &str) -> bool {
    if method == "POST" {
        return [
            "/instances/project/storage",
            flow_like_device_protocol::OFFLINE_REPLAY_PATH,
        ]
        .iter()
        .any(|path| endpoint_url(base, path).is_ok_and(|target| target == url));
    }
    if method != "GET" {
        return false;
    }
    if endpoint_url(base, "/instances/project/app").is_ok_and(|target| target == url) {
        return true;
    }
    let Ok(root) = endpoint_url(base, "/instances/project") else {
        return false;
    };
    let Some(path) = url.strip_prefix(&format!("{root}/")) else {
        return false;
    };
    let parts = path.split('/').collect::<Vec<_>>();
    (parts.len() == 6
        || (parts.len() == 8
            && parts[0] == "boards"
            && parts[6] == "pages"
            && flow_like_device_protocol::validate_instance_identifier(parts[7]).is_ok()))
        && matches!(parts[0], "boards" | "events" | "widgets" | "templates")
        && flow_like_device_protocol::validate_instance_identifier(parts[1]).is_ok()
        && parts[2] == "versions"
        && parts[3..6].iter().all(|part| {
            part.parse::<u32>()
                .is_ok_and(|v| v < u32::MAX && v.to_string() == *part)
        })
}

impl RequestAuthorizer for WorkloadBroker {
    fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
        Some(match audience {
            ResourceAudience::HostedModels => format!("{}/instances", self.base()),
            ResourceAudience::ProjectApi => format!("{}/instances/project", self.base()),
        })
    }

    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
        Box::pin(async move {
            let allowed = match request.audience {
                ResourceAudience::HostedModels => {
                    self.validation.is_none()
                        && self
                            .config
                            .resource_grant
                            .as_ref()
                            .is_some_and(|g| g.billing_grant_id.is_some())
                        && allowed_model_request(self.base(), request.method, request.url)
                }
                ResourceAudience::ProjectApi => {
                    self.config.source == crate::config::ProjectSource::Online
                        && (self.validation.is_none() || request.method == "GET")
                        && allowed_project_request(self.base(), request.method, request.url)
                }
            };
            if !allowed {
                return Err(AuthorizationError::InvalidRequest);
            }
            self.authorize_inner(request.audience, request.method, request.url)
                .await
                .map_err(|error| {
                    if error.downcast_ref::<AuthorizationError>()
                        == Some(&AuthorizationError::Denied)
                        || matches!(
                            api_status(&error),
                            Some(
                                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
                            )
                        )
                    {
                        AuthorizationError::Denied
                    } else {
                        #[cfg(feature = "runtime")]
                        {
                            crate::online::authorization_error(&error)
                        }
                        #[cfg(not(feature = "runtime"))]
                        {
                            AuthorizationError::Unavailable
                        }
                    }
                })
        })
    }
}

pub async fn drain_retirements(
    state_dir: PathBuf,
    session: Arc<DeviceSession>,
    cancel: tokio_util::sync::CancellationToken,
    wake: Arc<tokio::sync::Notify>,
) -> Result<()> {
    loop {
        retire_pending_once(&state_dir, &session, &cancel).await?;
        tokio::select! { _ = cancel.cancelled() => return Ok(()), _ = wake.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(30)) => {} }
    }
}

async fn retire_pending_once(
    state_dir: &std::path::Path,
    session: &DeviceSession,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    StateStore::open(&state_dir.join("management.sqlite"))?
        .forget_expired_retirements(unix_time()?)?;
    let pending = StateStore::open(&state_dir.join("management.sqlite"))?.pending_retirements()?;
    use futures_util::StreamExt;
    let mut requests = futures_util::stream::iter(pending)
        .map(|id| async move {
            let result =
                tokio::time::timeout(Duration::from_secs(5), session.retire_instance(&id)).await;
            (id, result)
        })
        .buffer_unordered(4);
    loop {
        let Some((id, result)) = (tokio::select! { _ = cancel.cancelled() => return Ok(()), result = requests.next() => result })
        else {
            break;
        };
        if matches!(result, Ok(Ok(true))) {
            StateStore::open(&state_dir.join("management.sqlite"))?.forget_instance(&id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{
        DpopContext, Ed25519PublicKey, verify_dpop, verify_instance_possession,
        verify_instance_registration, verify_workload_assertion,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Clone)]
    struct ApiFixture {
        base: String,
        device_id: String,
        device_key: Ed25519PublicKey,
        receipt: Arc<Mutex<Option<InstanceReceipt>>>,
        registrations: Arc<AtomicUsize>,
        tokens: Arc<AtomicUsize>,
        deny: Arc<AtomicBool>,
        lose_registration: Arc<AtomicBool>,
        retirements: Arc<AtomicUsize>,
    }

    async fn register(
        axum::extract::State(api): axum::extract::State<ApiFixture>,
        axum::Json(request): axum::Json<InstanceRegistrationRequest>,
    ) -> Result<axum::Json<InstanceReceipt>, axum::http::StatusCode> {
        let endpoint = format!("{}/devices/{}/instances", api.base, api.device_id);
        let registration = verify_instance_registration(
            &request.registration_jws,
            &api.device_key,
            &api.device_id,
            &endpoint,
            unix_time().unwrap(),
        )
        .unwrap();
        verify_instance_possession(
            &request.possession_jws,
            &registration.workload_key,
            &registration.instance_id,
            &endpoint,
            &request.registration_jws,
            unix_time().unwrap(),
        )
        .unwrap();
        api.registrations.fetch_add(1, Ordering::SeqCst);
        let now = unix_time().unwrap();
        let receipt = InstanceReceipt {
            instance_id: registration.instance_id,
            purpose: registration.purpose,
            device_id: registration.device_id,
            grant_id: registration.grant_id,
            billing_grant_id: registration.billing_grant_id,
            workload_key: registration.workload_key,
            key_epoch: 1,
            registered_at: now,
            lease_expires_at: now + 600,
            registration_jws: request.registration_jws,
        };
        *api.receipt.lock().await = Some(receipt.clone());
        if api.lose_registration.swap(false, Ordering::SeqCst) {
            return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(axum::Json(receipt))
    }

    async fn token(
        axum::extract::State(api): axum::extract::State<ApiFixture>,
        axum::Json(request): axum::Json<InstanceTokenRequest>,
    ) -> Result<axum::Json<InstanceTokenResponse>, axum::http::StatusCode> {
        let receipt = api.receipt.lock().await.clone().unwrap();
        verify_workload_assertion(
            &request.client_assertion,
            &receipt.workload_key,
            &receipt.instance_id,
            &format!("{}/instances/{}/token", api.base, receipt.instance_id),
            unix_time().unwrap(),
        )
        .unwrap();
        let generation = api.tokens.fetch_add(1, Ordering::SeqCst);
        if api.deny.load(Ordering::SeqCst) {
            return Err(axum::http::StatusCode::FORBIDDEN);
        }
        let now = unix_time().unwrap();
        Ok(axum::Json(InstanceTokenResponse {
            access_token: format!("token-{generation}"),
            token_type: "DPoP".into(),
            expires_in: 300,
            expires_at: now + 300,
            dpop_nonce: format!("server-issued-nonce-{generation}"),
            lease_expires_at: now + 600,
        }))
    }

    async fn receipt(
        axum::extract::State(api): axum::extract::State<ApiFixture>,
        axum::Json(request): axum::Json<ReceiptRequest>,
    ) -> axum::Json<InstanceReceipt> {
        let receipt = api.receipt.lock().await.clone().unwrap();
        verify_workload_assertion(
            &request.client_assertion,
            &receipt.workload_key,
            &receipt.instance_id,
            &format!("{}/instances/{}/receipt", api.base, receipt.instance_id),
            unix_time().unwrap(),
        )
        .unwrap();
        axum::Json(receipt)
    }

    async fn project_token(
        axum::extract::State(api): axum::extract::State<ApiFixture>,
        axum::Json(request): axum::Json<InstanceTokenRequest>,
    ) -> axum::Json<InstanceTokenResponse> {
        let receipt = api.receipt.lock().await.clone().unwrap();
        verify_workload_assertion(
            &request.client_assertion,
            &receipt.workload_key,
            &receipt.instance_id,
            &format!(
                "{}/instances/{}/project-token",
                api.base, receipt.instance_id
            ),
            unix_time().unwrap(),
        )
        .unwrap();
        let now = unix_time().unwrap();
        axum::Json(InstanceTokenResponse {
            access_token: "project-token".into(),
            token_type: "DPoP".into(),
            expires_in: 300,
            expires_at: now + 300,
            dpop_nonce: "project-server-issued-nonce".into(),
            lease_expires_at: now + 600,
        })
    }

    async fn retire(
        axum::extract::State(api): axum::extract::State<ApiFixture>,
        axum::Json(request): axum::Json<ReceiptRequest>,
    ) -> axum::http::StatusCode {
        let receipt = api.receipt.lock().await.clone().unwrap();
        flow_like_device_protocol::verify_client_assertion(
            &request.client_assertion,
            &api.device_key,
            &api.device_id,
            &format!(
                "{}/devices/{}/instances/{}",
                api.base, api.device_id, receipt.instance_id
            ),
            unix_time().unwrap(),
        )
        .unwrap();
        if api.retirements.fetch_add(1, Ordering::SeqCst) == 0 {
            axum::http::StatusCode::NOT_FOUND
        } else {
            axum::http::StatusCode::NO_CONTENT
        }
    }

    #[tokio::test]
    async fn broker_is_lazy_recovers_lost_registration_and_refreshes_retained_requests()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let base = format!("http://{}/api/v1", listener.local_addr()?);
        let device_key = SigningKey::generate();
        let api = ApiFixture {
            base: base.clone(),
            device_id: Uuid::new_v4().to_string(),
            device_key: device_key.public_key(),
            receipt: Arc::new(Mutex::new(None)),
            registrations: Arc::new(AtomicUsize::new(0)),
            tokens: Arc::new(AtomicUsize::new(0)),
            deny: Arc::new(AtomicBool::new(false)),
            lose_registration: Arc::new(AtomicBool::new(true)),
            retirements: Arc::new(AtomicUsize::new(0)),
        };
        let router = axum::Router::new()
            .route(
                "/api/v1/devices/{device}/instances",
                axum::routing::post(register),
            )
            .route(
                "/api/v1/devices/{device}/instances/{instance}",
                axum::routing::delete(retire),
            )
            .route(
                "/api/v1/instances/{instance}/token",
                axum::routing::post(token),
            )
            .route(
                "/api/v1/instances/{instance}/project-token",
                axum::routing::post(project_token),
            )
            .route(
                "/api/v1/instances/{instance}/receipt",
                axum::routing::post(receipt),
            )
            .with_state(api.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let session = Arc::new(DeviceSession::test_session(
            base.clone(),
            api.device_id.clone(),
            device_key,
        ));
        let config: PlacementConfig = serde_json::from_value(
            serde_json::json!({"id":"placement","project_id":"offline-project",
            "deployment_id":"deployment","revision":"one","source":"online","project_path":directory.path(),
            "events":[{"event_id":"daemon","event_version":[1,0,0],"board_version":[1,0,0]}],
            "resource_grant":{"grant_id":"grant","authz_version":1,"billing_grant_id":"billing","billing_authz_version":1}}),
        )?;
        let broker = Arc::new(WorkloadBroker::new(
            session,
            config,
            directory.path().into(),
            1,
            1,
        )?);
        assert_eq!(api.registrations.load(Ordering::SeqCst), 0);
        let endpoint = format!("{base}/instances/responses");
        assert!(
            broker
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::ProjectApi,
                    method: "POST",
                    url: &endpoint
                })
                .await
                .is_err()
        );
        assert_eq!(api.registrations.load(Ordering::SeqCst), 0);
        assert!(
            broker
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: &endpoint
                })
                .await
                .is_err()
        );
        assert_eq!(api.registrations.load(Ordering::SeqCst), 1);
        // An immutable receipt remains recoverable even after its old admission
        // lease expired. The token endpoint independently reacquires capacity.
        {
            let mut receipt = api.receipt.lock().await;
            let receipt = receipt.as_mut().unwrap();
            receipt.registered_at = unix_time()? - 700;
            receipt.lease_expires_at = unix_time()? - 100;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let broker = broker.clone();
            let endpoint = endpoint.clone();
            tasks.spawn(async move {
                broker
                    .authorize(AuthorizationRequest {
                        audience: ResourceAudience::HostedModels,
                        method: "POST",
                        url: &endpoint,
                    })
                    .await
                    .unwrap()
            });
        }
        let mut proof_ids = std::collections::HashSet::new();
        let receipt = api.receipt.lock().await.clone().unwrap();
        while let Some(result) = tasks.join_next().await {
            let authorization = result?;
            assert_eq!(authorization.authorization(), "DPoP token-0");
            let proof = verify_dpop(
                authorization.dpop().unwrap(),
                &receipt.workload_key,
                &DpopContext {
                    method: "POST",
                    url: &endpoint,
                    access_token: Some("token-0"),
                    nonce: Some("server-issued-nonce-0"),
                    key_thumbprint: &receipt.workload_key.thumbprint()?,
                    now: unix_time()?,
                },
            )?;
            assert!(proof_ids.insert(proof.jti));
        }
        assert_eq!(api.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(api.tokens.load(Ordering::SeqCst), 1);
        broker.state.lock().await.lease.as_mut().unwrap().refresh_at = 0;
        let authorization = broker
            .authorize(AuthorizationRequest {
                audience: ResourceAudience::HostedModels,
                method: "POST",
                url: &endpoint,
            })
            .await?;
        assert_eq!(authorization.authorization(), "DPoP token-1");
        api.deny.store(true, Ordering::SeqCst);
        broker.state.lock().await.lease.as_mut().unwrap().refresh_at = 0;
        assert_eq!(
            broker
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: &endpoint
                })
                .await
                .unwrap_err(),
            AuthorizationError::Denied
        );
        assert_eq!(
            broker
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: &endpoint
                })
                .await
                .unwrap_err(),
            AuthorizationError::Denied
        );
        assert_eq!(api.tokens.load(Ordering::SeqCst), 3);
        let project_endpoint = format!("{base}/instances/project/app");
        let project_authorization = broker
            .authorize(AuthorizationRequest {
                audience: ResourceAudience::ProjectApi,
                method: "GET",
                url: &project_endpoint,
            })
            .await?;
        assert_eq!(project_authorization.authorization(), "DPoP project-token");
        verify_dpop(
            project_authorization.dpop().unwrap(),
            &broker.key.public_key(),
            &DpopContext {
                method: "GET",
                url: &project_endpoint,
                access_token: Some("project-token"),
                nonce: Some("project-server-issued-nonce"),
                key_thumbprint: &broker.key.public_key().thumbprint()?,
                now: unix_time()?,
            },
        )?;
        // Cloud credentials can outlive the registration lease. After a quiet
        // hour, renew with the retained workload key instead of registering a
        // new instance or obtaining user credentials.
        {
            let mut state = broker.state.lock().await;
            let lease = state.project_lease.as_mut().unwrap();
            lease.expires_at = unix_time()? - 1;
            lease.refresh_at = 0;
            state.receipt.as_mut().unwrap().lease_expires_at = unix_time()? - 1800;
            api.receipt.lock().await.as_mut().unwrap().lease_expires_at = unix_time()? - 1800;
        }
        let storage_endpoint = format!("{base}/instances/project/storage");
        let authorization = broker
            .authorize(AuthorizationRequest {
                audience: ResourceAudience::ProjectApi,
                method: "POST",
                url: &storage_endpoint,
            })
            .await?;
        assert_eq!(authorization.authorization(), "DPoP project-token");
        assert_eq!(api.registrations.load(Ordering::SeqCst), 1);
        assert!(authorization.expires_at() > std::time::SystemTime::now());
        let store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.retire_instance(broker.instance_id())?;
        retire_pending_once(
            directory.path(),
            &broker.device,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await?;
        assert_eq!(store.pending_retirements()?, [broker.instance_id()]);
        retire_pending_once(
            directory.path(),
            &broker.device,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await?;
        assert!(store.pending_retirements()?.is_empty());
        server.abort();
        Ok(())
    }

    #[cfg(feature = "runtime")]
    #[tokio::test]
    async fn outage_mac_survives_broker_restart_but_denial_key_and_config_changes_do_not()
    -> Result<()> {
        use crate::online::outage::{OutageAuthority, SnapshotClaim};
        use flow_like_device_protocol::{
            InstanceStorageCredential, InstanceStorageLocation, OnlineProjectAccess, StoragePurpose,
        };
        let directory = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/api/v1", listener.local_addr()?);
        let stored_lease: Arc<Mutex<Option<InstanceStorageLease>>> = Arc::new(Mutex::new(None));
        let registration_status = Arc::new(std::sync::atomic::AtomicU16::new(503));
        let fallback_status = registration_status.clone();
        let response = stored_lease.clone();
        let router =
            axum::Router::new()
                .route(
                    "/api/v1/instances/project/storage",
                    axum::routing::post(move || {
                        let response = response.clone();
                        async move { axum::Json(response.lock().await.clone().unwrap()) }
                    }),
                )
                .fallback(move || {
                    let status = fallback_status.clone();
                    async move {
                        axum::http::StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap()
                    }
                });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let session = Arc::new(DeviceSession::test_session(
            base.clone(),
            "device".into(),
            SigningKey::generate(),
        ));
        let config: PlacementConfig = serde_json::from_value(
            serde_json::json!({"id":"placement","project_id":"project","deployment_id":"deployment","revision":"one","source":"online","project_path":directory.path(),"events":[{"event_id":"daemon","event_version":[1,0,0],"board_version":[1,0,0]}],"resource_grant":{"grant_id":"grant","authz_version":1}}),
        )?;
        let broker = WorkloadBroker::new(
            session.clone(),
            config.clone(),
            directory.path().into(),
            1,
            1,
        )?;
        let mut claim = SnapshotClaim {
            binding: broker.outage_binding()?,
            digest: "a".repeat(64),
            storage_context_digest: "a".repeat(64),
            grant_expires_at: unix_time()? + 3600,
        };
        assert!(
            broker.seal(&claim).await.is_err(),
            "No offline authorization before a successful grant check"
        );
        broker.state.lock().await.project_lease = Some(ResourceLease {
            token: Zeroizing::new("ephemeral-token".into()),
            nonce: "n".repeat(43),
            expires_at: unix_time()? + 180,
            refresh_at: unix_time()? + 120,
        });
        broker.state.lock().await.receipt = Some(InstanceReceipt {
            instance_id: broker.instance_id().into(),
            purpose: InstancePurpose::Workload,
            device_id: "device".into(),
            grant_id: "grant".into(),
            billing_grant_id: None,
            workload_key: broker.key.public_key(),
            key_epoch: 1,
            registered_at: unix_time()?,
            lease_expires_at: unix_time()? + 600,
            registration_jws: "unit-fixture".into(),
        });
        let locations = [
            (StoragePurpose::Files, "apps/project/upload/"),
            (StoragePurpose::Storage, "apps/project/storage/"),
            (StoragePurpose::User, "users/owner/apps/project/"),
            (StoragePurpose::Temporary, "tmp/user/owner/apps/project/"),
        ]
        .into_iter()
        .map(|(purpose, prefix)| {
            (
                purpose,
                InstanceStorageLocation {
                    uri: format!("s3://content/{prefix}"),
                    prefix: prefix.into(),
                    credential_id: "device".into(),
                    options: std::collections::BTreeMap::new(),
                },
            )
        })
        .collect();
        let lease = InstanceStorageLease {
            instance_id: broker.instance_id().into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
            grant_id: "grant".into(),
            authz_version: 1,
            project_id: "project".into(),
            placement_id: "placement".into(),
            deployment_id: "deployment".into(),
            delegating_user_id: "owner".into(),
            access: OnlineProjectAccess::ReadOnly,
            expires_at: unix_time()? + 1800,
            grant_expires_at: Some(claim.grant_expires_at),
            locations,
            credentials: std::collections::BTreeMap::from([(
                "device".into(),
                InstanceStorageCredential::AwsSession {
                    access_key_id: "fixture".into(),
                    secret_access_key: "fixture".into(),
                    session_token: "fixture".into(),
                },
            )]),
        };
        *stored_lease.lock().await = Some(lease.clone());
        let mut path = directory.path().to_path_buf();
        for component in ["placement-data", "placement", "current", "store"] {
            path.push(component);
            crate::outbox::private_directory(&path)?;
        }
        crate::vault::write_new_private(
            &path.parent().unwrap().join("binding.json"),
            &serde_json::to_vec(
                &serde_json::json!({"version":1,"placement_id":"placement","project_id":"project","deployment_id":"deployment","source":"online","initial_snapshot":config.project_path}),
            )?,
        )?;
        for component in [".standalone-cache", "placement", "outage"] {
            path.push(component);
            crate::outbox::private_directory(&path)?;
        }
        path.push("snapshot.json");
        let scope = crate::online::cache_scope(
            &config,
            &broker.identity(),
            &lease,
            &format!("{base}/instances/project"),
        )?;
        let mut envelope = serde_json::json!({"snapshot":{"version":1,"binding":claim.binding,"context":{"locations":lease.locations,"delegating_user_id":"owner","access":"read_only","scope":scope,"grant_expires_at":claim.grant_expires_at},"metadata":[{"path":"apps/project/manifest.app","content":"AA=="}]},"seal":""});
        crate::vault::write_new_private(&path, &serde_json::to_vec(&envelope)?)?;
        claim = crate::online::outage::inspect_claim(&path, &claim.binding)?.0;
        let mut forged = claim.clone();
        forged.grant_expires_at += 86400;
        assert!(
            broker.seal(&forged).await.is_err(),
            "A child cannot extend the backend grant deadline"
        );
        let mut forged = claim.clone();
        forged.storage_context_digest = "c".repeat(64);
        assert!(
            broker.seal(&forged).await.is_err(),
            "A child cannot substitute user, access or cloud paths"
        );
        let seal = broker.seal(&claim).await?;
        envelope["seal"] = serde_json::json!(seal);
        std::fs::write(&path, serde_json::to_vec(&envelope)?)?;
        assert!(WorkloadBroker::can_restore_outage(
            &session,
            &config,
            directory.path()
        )?);
        std::fs::remove_file(&path)?;
        assert!(!WorkloadBroker::can_restore_outage(
            &session,
            &config,
            directory.path()
        )?);
        let mut corrupt = envelope.clone();
        corrupt["snapshot"]["context"]["delegating_user_id"] = serde_json::json!("stranger");
        crate::vault::write_new_private(&path, &serde_json::to_vec(&corrupt)?)?;
        assert!(!WorkloadBroker::can_restore_outage(
            &session,
            &config,
            directory.path()
        )?);
        std::fs::write(&path, serde_json::to_vec(&envelope)?)?;
        let old_claim = claim.clone();
        let old_seal = seal.clone();
        let mut extended = stored_lease.lock().await.clone().unwrap();
        extended.grant_expires_at = Some(claim.grant_expires_at + 600);
        *stored_lease.lock().await = Some(extended);
        envelope["snapshot"]["context"]["grant_expires_at"] =
            serde_json::json!(claim.grant_expires_at + 600);
        std::fs::write(&path, serde_json::to_vec(&envelope)?)?;
        claim = crate::online::outage::inspect_claim(&path, &claim.binding)?.0;
        let seal = broker.seal(&claim).await?;
        assert!(broker.verify(&old_claim, &old_seal).await.is_err());
        broker.verify(&claim, &seal).await?;
        envelope["seal"] = serde_json::json!(seal);
        std::fs::write(&path, serde_json::to_vec(&envelope)?)?;
        broker.state.lock().await.ready = true;
        assert!(
            broker.seal(&claim).await.is_err(),
            "User workflows cannot ask the supervisor to seal new state"
        );
        drop(broker);
        let restarted = WorkloadBroker::new(
            session.clone(),
            config.clone(),
            directory.path().into(),
            1,
            1,
        )?;
        restarted.verify(&claim, &seal).await?;
        assert!(
            restarted.state.lock().await.project_lease.is_none(),
            "Offline verification never invents a cloud token"
        );
        restarted.prepare().await?;
        let mut changed = claim.clone();
        changed.digest = "b".repeat(64);
        assert!(restarted.verify(&changed, &seal).await.is_err());
        let mut changed_config = config.clone();
        changed_config.revision = "two".into();
        let changed = WorkloadBroker::new(
            session.clone(),
            changed_config,
            directory.path().into(),
            1,
            1,
        )?;
        assert!(changed.verify(&claim, &seal).await.is_err());
        let changed_key = Arc::new(DeviceSession::test_session(
            base,
            "device".into(),
            SigningKey::generate(),
        ));
        let changed =
            WorkloadBroker::new(changed_key, config.clone(), directory.path().into(), 1, 1)?;
        assert!(changed.verify(&claim, &seal).await.is_err());
        registration_status.store(403, Ordering::SeqCst);
        assert!(restarted.prepare().await.is_err());
        assert!(
            !WorkloadBroker::can_restore_outage(&session, &config, directory.path())?,
            "Definitive denial during final readiness must fence the next offline restart"
        );
        assert!(
            !path.exists(),
            "Final readiness denial must wipe the persisted snapshot"
        );
        restarted.deny(&claim.binding).await?;
        assert!(!WorkloadBroker::can_restore_outage(
            &session,
            &config,
            directory.path()
        )?);
        drop(restarted);
        let revoked = WorkloadBroker::new(session, config, directory.path().into(), 1, 1)?;
        assert_eq!(
            revoked
                .verify(&claim, &seal)
                .await
                .unwrap_err()
                .downcast::<AuthorizationError>()?,
            AuthorizationError::Denied
        );
        server.abort();
        Ok(())
    }
    #[test]
    fn project_authorization_has_a_fixed_read_and_lease_surface() {
        let base = "https://api.example/api/v1";
        for path in [
            "/instances/project/app",
            "/instances/project/boards/board/versions/1/2/3",
            "/instances/project/events/event/versions/0/1/0",
            "/instances/project/boards/board/versions/1/2/3/pages/page",
        ] {
            assert!(allowed_project_request(
                base,
                "GET",
                &format!("{base}{path}")
            ));
            assert!(!allowed_project_request(
                base,
                "DELETE",
                &format!("{base}{path}")
            ));
        }
        assert!(allowed_project_request(
            base,
            "POST",
            &format!("{base}/instances/project/storage")
        ));
        for path in [
            "/instances/project/storage/user",
            "/instances/project/app?project=other",
            "/instances/project/boards/board/versions/01/2/3",
            "/instances/project/boards/../versions/1/2/3",
            "/instances/project/boards/board/versions/1/2/3/pages/other/extra",
            "/instances/project/boards/board/versions/1/2/3/pages/page?app=other",
            "/instances/project/widgets/widget/versions/1/2/3/pages/page",
            "/instances/chat/completions",
            "/apps/other",
        ] {
            assert!(!allowed_project_request(
                base,
                "GET",
                &format!("{base}{path}")
            ));
            assert!(!allowed_project_request(
                base,
                "POST",
                &format!("{base}{path}")
            ));
        }
    }

    #[tokio::test]
    async fn validation_broker_is_metadata_only_and_fences_inflight_and_cached_credentials()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let base = format!("http://{}/api/v1", listener.local_addr()?);
        let device_key = SigningKey::generate();
        let api = ApiFixture {
            base: base.clone(),
            device_id: Uuid::new_v4().to_string(),
            device_key: device_key.public_key(),
            receipt: Arc::new(Mutex::new(None)),
            registrations: Arc::new(AtomicUsize::new(0)),
            tokens: Arc::new(AtomicUsize::new(0)),
            deny: Arc::new(AtomicBool::new(false)),
            lose_registration: Arc::new(AtomicBool::new(false)),
            retirements: Arc::new(AtomicUsize::new(0)),
        };
        let block = Arc::new(AtomicBool::new(false));
        let requested = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let handler_block = block.clone();
        let handler_requested = requested.clone();
        let handler_release = release.clone();
        let router = axum::Router::new()
            .route(
                "/api/v1/devices/{device}/instances",
                axum::routing::post(register),
            )
            .route(
                "/api/v1/devices/{device}/instances/{instance}",
                axum::routing::delete(retire),
            )
            .route(
                "/api/v1/instances/{instance}/project-token",
                axum::routing::post(move |api, request| {
                    let block = handler_block.clone();
                    let requested = handler_requested.clone();
                    let release = handler_release.clone();
                    async move {
                        if block.load(Ordering::SeqCst) {
                            requested.notify_one();
                            release.notified().await;
                        }
                        project_token(api, request).await
                    }
                }),
            )
            .with_state(api.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let session = Arc::new(DeviceSession::test_session(
            base.clone(),
            api.device_id.clone(),
            device_key,
        ));
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"service","project_id":"project","deployment_id":"deployment","revision":"one",
            "source":"online","project_path":directory.path(),
            "hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"auth"},
            "events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}],
            "resource_grant":{"grant_id":"grant","authz_version":1,"billing_grant_id":"billing","billing_authz_version":1}
        }))?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement(
            "service",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        let mut candidate = config.clone();
        candidate.revision = "two".into();
        let now = unix_time()?;
        store.stage_rollout("rollout", &candidate, 1, 2, 120, now)?;
        store.begin_rollout_validation("rollout", now)?;
        let broker = Arc::new(WorkloadBroker::new_rollout_validation(
            session.clone(),
            candidate.clone(),
            directory.path().into(),
            "rollout".into(),
        )?);
        let mut unbound = candidate;
        unbound.revision = "unapproved".into();
        assert!(
            WorkloadBroker::new_rollout_validation(
                session,
                unbound,
                directory.path().into(),
                "rollout".into()
            )
            .is_err()
        );
        for (audience, method, path) in [
            (
                ResourceAudience::HostedModels,
                "POST",
                "/instances/responses",
            ),
            (
                ResourceAudience::ProjectApi,
                "POST",
                "/instances/project/storage/files",
            ),
        ] {
            assert_eq!(
                broker
                    .authorize(AuthorizationRequest {
                        audience,
                        method,
                        url: &format!("{base}{path}")
                    })
                    .await
                    .unwrap_err(),
                AuthorizationError::InvalidRequest
            );
        }
        assert_eq!(api.registrations.load(Ordering::SeqCst), 0);
        let endpoint = format!("{base}/instances/project/app");
        let authorize = || {
            broker.authorize(AuthorizationRequest {
                audience: ResourceAudience::ProjectApi,
                method: "GET",
                url: &endpoint,
            })
        };
        assert_eq!(authorize().await?.authorization(), "DPoP project-token");
        let receipt = api.receipt.lock().await.clone().unwrap();
        assert_eq!(receipt.purpose, InstancePurpose::RolloutValidation);
        assert_eq!(receipt.billing_grant_id, None);
        let mut substituted = receipt.clone();
        substituted.purpose = InstancePurpose::Workload;
        let mut receipt_state = BrokerState {
            registrations: vec![receipt.registration_jws.clone()],
            ..Default::default()
        };
        assert!(
            broker
                .accept_receipt(&mut receipt_state, substituted)
                .is_err()
        );
        broker.accept_receipt(&mut receipt_state, receipt)?;
        let binding: String = store.connection.query_row(
            "SELECT launch_binding_json FROM workload_instances WHERE instance_id=?1",
            [broker.instance_id()],
            |r| r.get(0),
        )?;
        let binding: serde_json::Value = serde_json::from_str(&binding)?;
        assert_eq!(binding["rollout_id"], "rollout");
        assert!(binding.get("replica_slot").is_none());
        // Even a cached, unexpired lease cannot outlive its durable rollout deadline.
        store.connection.execute(
            "UPDATE placement_rollouts SET deadline_at=?1 WHERE rollout_id='rollout'",
            [unix_time()?],
        )?;
        assert_eq!(authorize().await.unwrap_err(), AuthorizationError::Denied);
        store.connection.execute(
            "UPDATE placement_rollouts SET deadline_at=?1 WHERE rollout_id='rollout'",
            [unix_time()? + 120],
        )?;
        block.store(true, Ordering::SeqCst);
        broker
            .state
            .lock()
            .await
            .project_lease
            .as_mut()
            .unwrap()
            .refresh_at = 0;
        let pending_broker = broker.clone();
        let pending_url = endpoint.clone();
        let pending = tokio::spawn(async move {
            pending_broker
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::ProjectApi,
                    method: "GET",
                    url: &pending_url,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), requested.notified()).await?;
        store.set_desired_state("service", crate::state::DesiredState::Stopped)?;
        release.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), pending)
                .await??
                .unwrap_err(),
            AuthorizationError::Denied
        );
        assert_eq!(authorize().await.unwrap_err(), AuthorizationError::Denied);
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        let id = broker.instance_id().to_owned();
        drop(broker);
        assert!(store.pending_retirements()?.contains(&id));
        server.abort();
        Ok(())
    }

    #[test]
    fn workload_authorization_accepts_only_exact_hosted_model_posts() {
        let base = "https://api.example/api/v1";
        assert!(allowed_model_request(
            base,
            "POST",
            "https://api.example/api/v1/instances/responses"
        ));
        for url in [
            "https://evil.example/api/v1/instances/responses",
            "https://api.example/api/v1/instances/responses?x=1",
            "https://api.example/api/v1/instances/../responses",
            "https://api.example/api/v1/chat/completions",
            "https://api.example/api/v1/instances/embeddings/embed/",
            "https://api.example/api/v1/instances/responses#x",
        ] {
            assert!(!allowed_model_request(base, "POST", url));
        }
        assert!(!allowed_model_request(
            base,
            "GET",
            "https://api.example/api/v1/instances/responses"
        ));
    }
}
