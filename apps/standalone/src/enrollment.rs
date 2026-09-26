use crate::{
    state::{RegistrationRecord, StateStore},
    supervisor, vault,
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::*;
use rand_core::{OsRng, RngCore};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const MAX_ENROLLMENT_BINDING_ATTEMPTS: usize = 64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentPackage {
    manifest: OnboardingManifest,
    manifest_jws: String,
    enrollment_token: String,
    bootstrap_secret: [u8; 32],
}

impl Drop for EnrollmentPackage {
    fn drop(&mut self) {
        self.bootstrap_secret.zeroize();
        self.enrollment_token.zeroize();
    }
}

pub struct DeviceKeys {
    auth: SigningKey,
    telemetry: SigningKey,
    management: Zeroizing<[u8; 32]>,
}

impl DeviceKeys {
    fn load_or_create(state_dir: &Path) -> Result<Self> {
        let path = state_dir.join("device.keys");
        if !path.try_exists()? {
            let mut bytes = Zeroizing::new([0; 96]);
            OsRng.fill_bytes(bytes.as_mut());
            vault::write_new_private(&path, bytes.as_ref())?;
        }
        Self::load(state_dir)
    }

    fn load(state_dir: &Path) -> Result<Self> {
        let bytes = vault::read_private(&state_dir.join("device.keys"))?;
        ensure!(bytes.len() == 96, "Malformed device key file");
        Ok(Self {
            auth: SigningKey::from_bytes(bytes[..32].try_into()?),
            telemetry: SigningKey::from_bytes(bytes[32..64].try_into()?),
            management: Zeroizing::new(bytes[64..].try_into()?),
        })
    }

    pub fn identity(&self) -> DeviceIdentity {
        let secret = x25519_dalek::StaticSecret::from(*self.management);
        DeviceIdentity {
            auth_key: self.auth.public_key(),
            telemetry_key: self.telemetry.public_key(),
            management_key: x25519_dalek::PublicKey::from(&secret).to_bytes(),
        }
    }

    fn assertion(&self, device_id: &str, endpoint: &str) -> Result<String> {
        let now = unix_time()?;
        Ok(sign_client_assertion(
            &ClientAssertion {
                iss: device_id.into(),
                sub: device_id.into(),
                aud: endpoint.into(),
                iat: now,
                nbf: now,
                exp: now + 60,
                jti: Uuid::new_v4().to_string(),
            },
            &self.auth,
        )?)
    }

    fn dpop(&self, method: &Method, endpoint: &str, token: &str) -> Result<String> {
        Ok(sign_dpop(
            &DpopProof {
                jti: Uuid::new_v4().to_string(),
                htm: method.as_str().into(),
                htu: canonical_dpop_htu(endpoint)?,
                iat: unix_time()?,
                ath: Some(access_token_hash(token)),
                nonce: None,
            },
            &self.auth,
        )?)
    }
}

pub fn unix_time() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}

pub(crate) fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("flow-like-standalone/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

#[derive(Debug, thiserror::Error)]
#[error("Device API returned HTTP {0}")]
struct ApiStatus(StatusCode);

pub(crate) async fn response_json<T: DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T> {
    if !response.status().is_success() {
        return Err(ApiStatus(response.status()).into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "Device API response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).context("Invalid device API response")
}

pub(crate) fn api_status(error: &anyhow::Error) -> Option<StatusCode> {
    error.downcast_ref::<ApiStatus>().map(|error| error.0)
}

/// Explicit development export from a caller-supplied local binary.
pub async fn create_package(
    controller_dir: &Path,
    api_url: &str,
    name: &str,
    output: &Path,
    access_token: &str,
    password: &[u8],
    binary: &Path,
) -> Result<OnboardingManifest> {
    create_package_inner(
        controller_dir,
        api_url,
        name,
        output,
        access_token,
        password,
        Some(binary),
        None,
    )
    .await
}

pub async fn create_release_package(
    controller_dir: &Path,
    api_url: &str,
    name: &str,
    output: &Path,
    access_token: &str,
    password: &[u8],
    release: &crate::release::PreparedPackage,
) -> Result<OnboardingManifest> {
    create_package_inner(
        controller_dir,
        api_url,
        name,
        output,
        access_token,
        password,
        None,
        Some(release),
    )
    .await
}

async fn create_package_inner(
    controller_dir: &Path,
    api_url: &str,
    name: &str,
    output: &Path,
    access_token: &str,
    password: &[u8],
    binary: Option<&Path>,
    release: Option<&crate::release::PreparedPackage>,
) -> Result<OnboardingManifest> {
    ensure!(
        cfg!(any(target_os = "linux", target_os = "macos")),
        "Atomic deployment package export currently requires Linux or macOS"
    );
    let api_base_url = canonical_api_base_url(api_url)?;
    match std::fs::symlink_metadata(output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => anyhow::bail!("Package output already exists"),
    }
    ensure!(
        !access_token.trim().is_empty(),
        "User access token is empty"
    );
    ensure!(
        (12..=4096).contains(&password.len()),
        "Use a password between 12 and 4096 bytes"
    );
    let output_parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output_absolute = output_parent.canonicalize()?.join(
        output
            .file_name()
            .context("Package output needs a directory name")?,
    );
    let mut source_binary = binary
        .map(std::fs::File::open)
        .transpose()
        .context("Open standalone binary")?;
    if let Some(binary) = &source_binary {
        ensure!(
            binary.metadata()?.is_file(),
            "Standalone binary must be a regular file"
        );
    }
    let controller_dir = supervisor::prepare_state_dir(controller_dir)?;
    ensure!(
        !controller_dir.starts_with(&output_absolute),
        "Controller vault must remain outside the deployment package"
    );
    let mut stage =
        PackageStage::create(output_absolute.parent().context("Missing output parent")?)?;
    vault::write_new_private(&stage.path.join(".gitignore"), b"*\n!.gitignore\n")?;
    let package_state = supervisor::prepare_state_dir(&stage.path.join("state"))?;
    vault::write_new_private(
        &package_state.join("agent.env"),
        crate::isolation::OPERATOR_ENV_TEMPLATE.as_bytes(),
    )?;
    vault::write_new_private(
        &stage.path.join(".env"),
        b"FLOW_LIKE_STANDALONE_STATE_DIR=./state\nFLOW_LIKE_PUBLISH_HOST=127.0.0.1\nFLOW_LIKE_SERVICE_PORT=8080\n# Durable host isolation policy: edit ./state/agent.env (native service and Docker).\n# Read cache: 512 MiB per placement, shared by replicas. SQLite adds disk overhead.\n# Docker loads this setting. Export it when starting the binary directly.\nFLOW_LIKE_DEVICE_READ_CACHE_BYTES=536870912\n",
    )?;
    vault::write_new_private(
        &stage.path.join(".env.example"),
        b"FLOW_LIKE_STANDALONE_STATE_DIR=./state\nFLOW_LIKE_PUBLISH_HOST=127.0.0.1\nFLOW_LIKE_SERVICE_PORT=8080\n# Durable host isolation policy: edit ./state/agent.env (native service and Docker).\n# Read cache: 512 MiB per placement, shared by replicas. SQLite adds disk overhead.\n# Docker loads this setting. Export it when starting the binary directly.\nFLOW_LIKE_DEVICE_READ_CACHE_BYTES=536870912\n",
    )?;
    if let Some(release) = release {
        release.write_payload(&stage.path)?;
    } else {
        let mut binary_options = std::fs::OpenOptions::new();
        binary_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            binary_options.mode(0o700).custom_flags(libc::O_NOFOLLOW);
        }
        let mut executable = binary_options.open(stage.path.join("flow-like-standalone"))?;
        std::io::copy(
            source_binary
                .as_mut()
                .context("Missing development binary")?,
            &mut executable,
        )?;
        executable.sync_all()?;
        vault::write_new_private(
            &stage.path.join("platform.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"version":env!("CARGO_PKG_VERSION"),
            }))?,
        )?;
    }
    let controller = SigningKey::generate();
    let invitation = SigningKey::generate();
    let bootstrap = SigningKey::generate();
    let request = CreateEnrollmentRequest {
        name: name.into(),
        api_base_url: api_base_url.clone(),
        bootstrap_key: bootstrap.public_key(),
        controller_key: controller.public_key(),
        owner_invitation_key: invitation.public_key(),
    };
    let client = http_client()?;
    let response: CreateEnrollmentResponse = response_json(
        client
            .post(endpoint_url(&api_base_url, "/devices/enrollments")?)
            .bearer_auth(access_token)
            .json(&request)
            .send()
            .await?,
    )
    .await?;
    let manifest = response.manifest;
    let matches_requested_keys = manifest.bootstrap_key == request.bootstrap_key
        && manifest.controller_key == request.controller_key
        && manifest.owner_invitation_key == request.owner_invitation_key;
    let exported = (|| -> Result<()> {
        validate_manifest(&manifest)?;
        ensure!(
            manifest.api_base_url == request.api_base_url
                && manifest.name == request.name
                && matches_requested_keys,
            "Enrollment response changed the requested identity"
        );
        Uuid::parse_str(&manifest.device_id).context("Invalid device ID")?;
        Uuid::parse_str(&manifest.enrollment_id).context("Invalid enrollment ID")?;
        let manifest_jws = sign_manifest(&manifest, &controller)?;
        verify_manifest(&manifest_jws, &controller.public_key(), unix_time()?)?;
        let protected = vault::seal(
            password,
            &vault::controller_context(&manifest.device_id),
            &*Zeroizing::new(controller.to_bytes()),
        )?;
        vault::write_new_private(
            &controller_dir.join(format!("{}.vault", manifest.device_id)),
            &protected,
        )?;
        let protected_invitation = vault::seal(
            password,
            &vault::invitation_context(&manifest.device_id),
            &*Zeroizing::new(invitation.to_bytes()),
        )?;
        vault::write_new_private(
            &controller_dir.join(format!("{}.invitation.vault", manifest.device_id)),
            &protected_invitation,
        )?;
        vault::write_new_private(
            &controller_dir.join(format!("{}.json", manifest.device_id)),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        let package = EnrollmentPackage {
            manifest: manifest.clone(),
            manifest_jws,
            enrollment_token: response.enrollment_token,
            bootstrap_secret: bootstrap.to_bytes(),
        };
        let encoded = Zeroizing::new(serde_json::to_vec(&package)?);
        vault::write_new_private(&stage.path.join("onboarding.json"), &encoded)?;
        stage.publish(&output_absolute)?;
        Ok(())
    })();
    if let Err(error) = exported {
        if stage.published {
            return Err(error.context(format!(
                "Package is visible at {}, but export durability could not be confirmed. Enrollment {} and its local controller and invitation vaults were retained; inspect the existing package before retrying",
                output_absolute.display(), manifest.enrollment_id
            )));
        }
        // Only this request's newly reserved enrollment may be cancelled. Existing
        // controller files and a competing output directory are never removed.
        let cancellation = match (
            matches_requested_keys,
            Uuid::parse_str(&manifest.enrollment_id),
        ) {
            (true, Ok(id)) => {
                let endpoint = endpoint_url(&api_base_url, &format!("/devices/enrollments/{id}"))?;
                match client
                    .delete(&endpoint)
                    .bearer_auth(access_token)
                    .send()
                    .await
                {
                    Ok(response) if response.status() == StatusCode::NO_CONTENT => {
                        format!("Enrollment {id} was cancelled")
                    }
                    _ => format!(
                        "Could not confirm cancellation of enrollment {id}; cancel it with an authenticated DELETE {endpoint} before retrying"
                    ),
                }
            }
            (_, Err(_)) => {
                "The server returned an invalid enrollment ID; cancellation could not be attempted"
                    .into()
            }
            (false, Ok(id)) => format!(
                "Enrollment response {id} did not match this request's keys; cancellation was not attempted"
            ),
        };
        return Err(error.context(cancellation));
    }
    Ok(manifest)
}

struct PackageStage {
    path: PathBuf,
    published: bool,
}

impl PackageStage {
    fn create(parent: &Path) -> Result<Self> {
        let path = parent.join(format!(".flow-like-package-{}.tmp", Uuid::new_v4()));
        create_package_dir(&path)?;
        Ok(Self {
            path,
            published: false,
        })
    }

    fn publish(&mut self, output: &Path) -> Result<()> {
        self.publish_with_parent_sync(output, |parent| {
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })
    }

    fn publish_with_parent_sync(
        &mut self,
        output: &Path,
        sync_parent: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<()> {
        std::fs::File::open(&self.path)?.sync_all()?;
        rename_package_exclusive(&self.path, output)?;
        // Once visible, the package may already be used. A later durability error
        // must preserve its enrollment as well as its files.
        self.published = true;
        sync_parent(output.parent().context("Missing output parent")?)
            .context("Persist published package directory")?;
        Ok(())
    }
}

impl Drop for PackageStage {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn rename_package_exclusive(source: &Path, target: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())?;
        let target = CString::new(target.as_os_str().as_bytes())?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Publish package without replacing an existing output");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("Atomic deployment package export currently requires Linux or macOS")
}

fn create_package_dir(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .context("Create new deployment package directory")
}

fn check_receipt(
    record: &RegistrationRecord,
    identity: &DeviceIdentity,
    receipt: &DeviceReceipt,
) -> Result<()> {
    let known_binding = record.binding_jws.as_ref() == Some(&receipt.binding_jws)
        || record.attempted_bindings.contains(&receipt.binding_jws);
    ensure!(
        receipt.device_id == record.manifest.device_id
            && receipt.enrollment_id == record.manifest.enrollment_id
            && receipt.owner_id == record.manifest.owner_id
            && receipt.name == record.manifest.name
            && receipt.identity == *identity
            && receipt.manifest_jws == record.manifest_jws
            && known_binding,
        "Enrollment receipt does not match the persisted identity and signed binding"
    );
    Ok(())
}

fn remember_binding(record: &mut RegistrationRecord, binding: String) -> Result<()> {
    let mut history = record.attempted_bindings.clone();
    ensure!(
        history.len() <= MAX_ENROLLMENT_BINDING_ATTEMPTS,
        "Enrollment binding history exceeds its limit"
    );
    for candidate in record.binding_jws.iter().chain(std::iter::once(&binding)) {
        if !history.contains(candidate) {
            ensure!(
                history.len() < MAX_ENROLLMENT_BINDING_ATTEMPTS,
                "Enrollment reached 64 recorded attempts. Recover the existing registration or cancel this enrollment before creating a new package; preserve its recorded bindings"
            );
            history.push(candidate.clone());
        }
    }
    record.attempted_bindings = history;
    record.binding_jws = Some(binding);
    Ok(())
}

fn ensure_binding_capacity(record: &RegistrationRecord) -> Result<()> {
    let unrecorded_previous = usize::from(
        record
            .binding_jws
            .as_ref()
            .is_some_and(|binding| !record.attempted_bindings.contains(binding)),
    );
    ensure!(
        record.attempted_bindings.len() < MAX_ENROLLMENT_BINDING_ATTEMPTS - unrecorded_previous,
        "Enrollment reached 64 recorded attempts. Recover the existing registration or cancel this enrollment before creating a new package; preserve its recorded bindings"
    );
    Ok(())
}

async fn recover(
    client: &Client,
    record: &RegistrationRecord,
    keys: &DeviceKeys,
) -> Result<DeviceReceipt> {
    let endpoint = endpoint_url(
        &record.manifest.api_base_url,
        &format!("/devices/{}/receipt", record.manifest.device_id),
    )?;
    let receipt: DeviceReceipt = response_json(
        client
            .post(&endpoint)
            .json(&ReceiptRequest {
                client_assertion: keys.assertion(&record.manifest.device_id, &endpoint)?,
            })
            .send()
            .await?,
    )
    .await?;
    check_receipt(record, &keys.identity(), &receipt)?;
    Ok(receipt)
}

/// Redeem once. Before bootstrap cleanup, retrying recovers a lost response without rotating identity.
pub async fn enroll(state_dir: &Path, package_dir: &Path) -> Result<DeviceReceipt> {
    let _agent_lock = supervisor::lock_file(&state_dir.join("agent.lock"))?;
    let _enrollment_lock = supervisor::lock_file(&state_dir.join("enrollment.lock"))?;
    let package_bytes = vault::read_private(&package_dir.join("onboarding.json")).map_err(|error| {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            error.context(
                "The selected package has no onboarding.json. Successful enrollment removes this file; use recover-enrollment with the same --state-dir to recover an existing registration. For a new device, select an unconsumed enrollment package",
            )
        } else {
            error
        }
    })?;
    let package: EnrollmentPackage = serde_json::from_slice(&package_bytes)?;
    drop(package_bytes);
    let manifest = verify_manifest(
        &package.manifest_jws,
        &package.manifest.controller_key,
        package.manifest.issued_at,
    )?;
    ensure!(manifest == package.manifest, "Package manifest mismatch");
    let bootstrap = SigningKey::from_bytes(&package.bootstrap_secret);
    ensure!(
        bootstrap.public_key() == manifest.bootstrap_key,
        "Package bootstrap key mismatch"
    );
    Uuid::parse_str(&manifest.device_id).context("Invalid device ID")?;
    Uuid::parse_str(&manifest.enrollment_id).context("Invalid enrollment ID")?;
    let mut store = StateStore::open(&state_dir.join("management.sqlite"))?;
    if store.registration()?.is_some() {
        ensure!(
            state_dir.join("device.keys").try_exists()?,
            "Registered device keys are missing; refusing replacement"
        );
    } else {
        // Expired packages may recover a committed identity, but must not claim
        // a fresh local state directory or create replacement keys.
        verify_manifest(
            &package.manifest_jws,
            &package.manifest.controller_key,
            unix_time()?,
        )?;
    }
    let keys = DeviceKeys::load_or_create(state_dir)?;
    let identity = keys.identity();
    identity.validate()?;
    store.begin_registration(&RegistrationRecord {
        manifest,
        manifest_jws: package.manifest_jws.clone(),
        binding_jws: None,
        attempted_bindings: Vec::new(),
        receipt: None,
        last_contact_at: None,
        connection_status: "pending".into(),
    })?;
    let mut record = store.registration()?.context("Missing enrollment state")?;
    let client = http_client()?;
    if record.binding_jws.is_some() {
        match recover(&client, &record, &keys).await {
            Ok(receipt) => {
                record.receipt = Some(receipt.clone());
                record.connection_status = "enrolled".into();
                store.update_registration(&record)?;
                std::fs::remove_file(package_dir.join("onboarding.json")).context(
                    "Enrolled successfully, but could not remove consumed bootstrap credentials",
                )?;
                return Ok(receipt);
            }
            Err(error)
                if api_status(&error) == Some(StatusCode::NOT_FOUND)
                    && record.receipt.is_none() => {}
            Err(error) => return Err(error.context("Recover existing registration")),
        }
    }
    verify_manifest(
        &package.manifest_jws,
        &package.manifest.controller_key,
        unix_time()?,
    )?;
    // A replacement challenge invalidates the previous one. Preserve an in-flight
    // final attempt when there is no room to durably record another binding.
    ensure_binding_capacity(&record)?;
    let base = &record.manifest.api_base_url;
    let path = format!("/devices/enrollments/{}", record.manifest.enrollment_id);
    let challenge: EnrollmentChallenge = response_json(
        client
            .post(endpoint_url(base, &format!("{path}/challenge"))?)
            .json(&ChallengeRequest {
                enrollment_token: package.enrollment_token.clone(),
            })
            .send()
            .await?,
    )
    .await?;
    let now = unix_time()?;
    ensure!(challenge.expires_at > now, "Enrollment challenge expired");
    let expires_at = (now + 60)
        .min(challenge.expires_at)
        .min(record.manifest.expires_at);
    let manifest_digest = compact_digest(&record.manifest_jws);
    let binding = EnrollmentBinding {
        version: PROTOCOL_VERSION,
        enrollment_id: record.manifest.enrollment_id.clone(),
        device_id: record.manifest.device_id.clone(),
        identity,
        manifest_digest: manifest_digest.clone(),
        challenge_id: challenge.challenge_id.clone(),
        challenge_nonce: challenge.nonce.clone(),
        issued_at: now,
        expires_at,
    };
    let binding_jws = sign_binding(&binding, &bootstrap)?;
    let endpoint = endpoint_url(base, &format!("{path}/redeem"))?;
    let proof_jws = sign_enrollment_proof(
        &EnrollmentProof {
            iss: record.manifest.device_id.clone(),
            sub: record.manifest.device_id.clone(),
            aud: endpoint.clone(),
            iat: now,
            nbf: now,
            exp: expires_at,
            jti: Uuid::new_v4().to_string(),
            challenge_id: challenge.challenge_id,
            challenge_nonce: challenge.nonce,
            binding_digest: compact_digest(&binding_jws),
            manifest_digest,
        },
        &keys.auth,
    )?;
    remember_binding(&mut record, binding_jws.clone())?;
    store.update_registration(&record)?;
    let receipt: DeviceReceipt = response_json(
        client
            .post(endpoint)
            .json(&RedeemEnrollmentRequest {
                enrollment_token: package.enrollment_token.clone(),
                manifest_jws: package.manifest_jws.clone(),
                binding_jws,
                proof_jws,
            })
            .send()
            .await?,
    )
    .await?;
    check_receipt(&record, &keys.identity(), &receipt)?;
    record.receipt = Some(receipt.clone());
    record.connection_status = "enrolled".into();
    store.update_registration(&record)?;
    std::fs::remove_file(package_dir.join("onboarding.json"))
        .context("Enrolled successfully, but could not remove consumed bootstrap credentials")?;
    Ok(receipt)
}

/// Recheck registration using the permanent key, even after bootstrap credentials expired.
pub async fn recover_registration(state_dir: &Path) -> Result<DeviceReceipt> {
    let _agent_lock = supervisor::lock_file(&state_dir.join("agent.lock"))?;
    let _enrollment_lock = supervisor::lock_file(&state_dir.join("enrollment.lock"))?;
    let mut store = StateStore::open(&state_dir.join("management.sqlite"))?;
    let mut record = store
        .registration()?
        .context("Device enrollment has not started")?;
    let keys = DeviceKeys::load(state_dir)?;
    let receipt = recover(&http_client()?, &record, &keys).await?;
    record.receipt = Some(receipt.clone());
    record.connection_status = "enrolled".into();
    store.update_registration(&record)?;
    Ok(receipt)
}

struct SessionLease {
    token: Zeroizing<String>,
    expires_at: i64,
}

/// Session tokens stay in memory. Every request receives its own proof of possession.
pub struct DeviceSession {
    client: Client,
    manifest: OnboardingManifest,
    keys: DeviceKeys,
    lease: Mutex<Option<SessionLease>>,
    auth_epoch: u64,
}

impl DeviceSession {
    #[cfg(test)]
    pub(crate) fn test_management_session(
        api_base_url: String,
        device_id: String,
        auth: SigningKey,
        controller_public_key: Ed25519PublicKey,
    ) -> Self {
        let mut session = Self::test_session(api_base_url, device_id, auth);
        session.manifest.controller_key = controller_public_key;
        session
    }
    #[cfg(test)]
    pub(crate) fn test_with_invitation_key(mut self, key: Ed25519PublicKey) -> Self {
        self.manifest.owner_invitation_key = key;
        self
    }
    #[cfg(test)]
    pub(crate) fn test_session(api_base_url: String, device_id: String, auth: SigningKey) -> Self {
        let now = unix_time().unwrap();
        Self {
            client: http_client().unwrap(),
            manifest: OnboardingManifest {
                version: PROTOCOL_VERSION,
                enrollment_id: Uuid::new_v4().to_string(),
                device_id,
                owner_id: "owner".into(),
                name: "Broker fixture".into(),
                api_base_url,
                bootstrap_key: SigningKey::generate().public_key(),
                controller_key: SigningKey::generate().public_key(),
                owner_invitation_key: SigningKey::generate().public_key(),
                issued_at: now,
                expires_at: now + 3600,
            },
            keys: DeviceKeys {
                auth,
                telemetry: SigningKey::generate(),
                management: Zeroizing::new([42; 32]),
            },
            lease: Mutex::new(None),
            auth_epoch: 1,
        }
    }

    pub fn load(state_dir: &Path) -> Result<Option<Self>> {
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let Some(record) = store.registration()? else {
            return Ok(None);
        };
        let Some(receipt) = &record.receipt else {
            return Ok(None);
        };
        let keys = DeviceKeys::load(state_dir)?;
        check_receipt(&record, &keys.identity(), receipt)?;
        Ok(Some(Self {
            client: http_client()?,
            manifest: record.manifest,
            keys,
            lease: Mutex::new(None),
            auth_epoch: receipt.auth_epoch,
        }))
    }

    pub fn manifest(&self) -> &OnboardingManifest {
        &self.manifest
    }
    pub(crate) fn auth_epoch(&self) -> u64 {
        self.auth_epoch
    }

    pub(crate) fn telemetry_signer(&self) -> SigningKey {
        SigningKey::from_bytes(&self.keys.telemetry.to_bytes())
    }

    pub(crate) fn noise_responder(
        &self,
        peer: &[u8; 32],
        session_id: &str,
    ) -> Result<crate::crypto::noise::Handshake> {
        Ok(crate::crypto::noise::Handshake::responder(
            &self.keys.management,
            *peer,
            &self.manifest.device_id,
            session_id,
        )?)
    }

    pub(crate) async fn signaling(&self) -> Result<DeviceSignalingResponse> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/signaling/device", self.manifest.device_id),
        )?;
        let response: DeviceSignalingResponse = response_json(
            self.client
                .post(&endpoint)
                .json(&DeviceSignalingRequest {
                    client_assertion: self.keys.assertion(&self.manifest.device_id, &endpoint)?,
                })
                .send()
                .await?,
        )
        .await?;
        let now = unix_time()?;
        ensure!(
            response.device_auth_epoch == self.auth_epoch
                && response.expires_at > now + 30
                && response.expires_at <= now + 305
                && response.signaling_urls.len() <= 4,
            "Invalid signaling admission"
        );
        for endpoint in &response.signaling_urls {
            let url = url::Url::parse(endpoint)?;
            ensure!(
                url.scheme() == "wss"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path().ends_with("/ws/devices"),
                "Invalid signaling endpoint"
            );
        }
        Ok(response)
    }

    pub(crate) async fn management_policies(&self, after_version: u64) -> Result<Vec<String>> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/management/policies", self.manifest.device_id),
        )?;
        response_json(self.client.post(&endpoint).json(&serde_json::json!({"client_assertion":self.keys.assertion(&self.manifest.device_id,&endpoint)?,"after_version":after_version})).send().await?).await
    }

    pub(crate) async fn fleet_recipients(
        &self,
    ) -> Result<flow_like_device_protocol::FleetRecipients> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/fleet/recipients", self.manifest.device_id),
        )?;
        response_json(self.client.post(&endpoint).json(&serde_json::json!({"client_assertion":self.keys.assertion(&self.manifest.device_id,&endpoint)?})).send().await?).await
    }

    pub(crate) async fn upload_fleet(
        &self,
        bundle: &flow_like_device_protocol::EncryptedFleetSnapshot,
    ) -> Result<flow_like_device_protocol::FleetHead> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/fleet/snapshots", self.manifest.device_id),
        )?;
        response_json(self.client.post(&endpoint).json(&serde_json::json!({"client_assertion":self.keys.assertion(&self.manifest.device_id,&endpoint)?,"bundle":bundle})).send().await?).await
    }

    pub(crate) async fn publish_certificate_inventory(
        &self,
        inventory: &CertificateInventory,
    ) -> Result<()> {
        ensure!(
            inventory.device_id == self.manifest.device_id,
            "Certificate inventory device mismatch"
        );
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/certificate-inventory", self.manifest.device_id),
        )?;
        let (authorization, proof) = self.authorization(&Method::PUT, &endpoint).await?;
        let mut authorization = reqwest::header::HeaderValue::from_str(&authorization)?;
        authorization.set_sensitive(true);
        let mut proof = reqwest::header::HeaderValue::from_str(&proof)?;
        proof.set_sensitive(true);
        let mut current = inventory.clone();
        current.issued_at = unix_time()?;
        let compact = sign_certificate_inventory(&current, &self.keys.auth)?;
        let response = self
            .client
            .put(endpoint)
            .header("authorization", authorization)
            .header("dpop", proof)
            .json(&serde_json::json!({"inventory_jws":compact}))
            .send()
            .await?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            *self.lease.lock().await = None;
        }
        #[derive(Deserialize)]
        struct Receipt {
            revision: u64,
            certificates: Vec<CertificateInventoryEntry>,
        }
        let receipt: Receipt = response_json(response).await?;
        ensure!(
            receipt.revision == inventory.revision
                && receipt.certificates == inventory.certificates,
            "Certificate inventory acknowledgement changed the published revision"
        );
        Ok(())
    }

    pub(crate) async fn upload_archive(
        &self,
        bundle: &flow_like_device_protocol::EncryptedArchive,
    ) -> Result<()> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/archives", self.manifest.device_id),
        )?;
        let response:serde_json::Value=response_json(self.client.post(&endpoint).json(&serde_json::json!({"client_assertion":self.keys.assertion(&self.manifest.device_id,&endpoint)?,"bundle":bundle})).send().await?).await?;
        ensure!(
            response["manifest_digest"].as_str()
                == Some(flow_like_device_protocol::compact_digest(&bundle.manifest_jws).as_str()),
            "Archive receipt digest mismatch"
        );
        Ok(())
    }

    pub(crate) async fn acknowledge_policy(&self, version: u64, digest: &str) -> Result<()> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/management/applied", self.manifest.device_id),
        )?;
        let _: serde_json::Value = response_json(self.client.post(&endpoint).json(&serde_json::json!({"client_assertion":self.keys.assertion(&self.manifest.device_id,&endpoint)?,"version":version,"digest":digest})).send().await?).await?;
        Ok(())
    }

    pub(crate) fn sign_instance_registration(
        &self,
        mut registration: InstanceRegistration,
    ) -> Result<String> {
        ensure!(
            registration.device_id == self.manifest.device_id,
            "Device registration identity mismatch"
        );
        registration.device_auth_epoch = self.auth_epoch;
        Ok(flow_like_device_protocol::sign_instance_registration(
            &registration,
            &self.keys.auth,
        )?)
    }

    pub(crate) async fn retire_instance(&self, instance_id: &str) -> Result<bool> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!(
                "/devices/{}/instances/{instance_id}",
                self.manifest.device_id
            ),
        )?;
        let response = self
            .client
            .delete(&endpoint)
            .json(&ReceiptRequest {
                client_assertion: self.keys.assertion(&self.manifest.device_id, &endpoint)?,
            })
            .send()
            .await?;
        ensure!(
            response.status().is_success() || response.status() == StatusCode::NOT_FOUND,
            "Instance retirement was not accepted"
        );
        // A missing instance may still be committing an earlier registration.
        // The caller must retain that uncertain admission until its lease bound.
        Ok(response.status().is_success())
    }

    async fn authorization(
        &self,
        method: &Method,
        endpoint: &str,
    ) -> Result<(Zeroizing<String>, String)> {
        let mut lease = self.lease.lock().await;
        let now = unix_time()?;
        if lease
            .as_ref()
            .is_none_or(|lease| lease.expires_at <= now + 60)
        {
            *lease = None;
            let endpoint = endpoint_url(&self.manifest.api_base_url, "/devices/token")?;
            let mut response: DeviceTokenResponse = response_json(
                self.client
                    .post(&endpoint)
                    .json(&DeviceTokenRequest {
                        device_id: self.manifest.device_id.clone(),
                        client_assertion: self
                            .keys
                            .assertion(&self.manifest.device_id, &endpoint)?,
                    })
                    .send()
                    .await?,
            )
            .await?;
            ensure!(
                response.token_type == "DPoP"
                    && response.expires_in > 60
                    && response.expires_in <= 600
                    && response.expires_at > unix_time()? + 60
                    && response.expires_at <= unix_time()? + 605,
                "Invalid device session lease"
            );
            *lease = Some(SessionLease {
                token: Zeroizing::new(std::mem::take(&mut response.access_token)),
                expires_at: response.expires_at,
            });
        }
        let token = &lease.as_ref().context("Missing session lease")?.token;
        let proof = self.keys.dpop(method, endpoint, token)?;
        Ok((Zeroizing::new(format!("DPoP {}", &**token)), proof))
    }

    pub async fn heartbeat(&self, heartbeat: &DeviceHeartbeat) -> Result<()> {
        let endpoint = endpoint_url(
            &self.manifest.api_base_url,
            &format!("/devices/{}/heartbeat", self.manifest.device_id),
        )?;
        let (authorization, proof) = self.authorization(&Method::POST, &endpoint).await?;
        let mut authorization = reqwest::header::HeaderValue::from_str(&authorization)?;
        authorization.set_sensitive(true);
        let mut proof = reqwest::header::HeaderValue::from_str(&proof)?;
        proof.set_sensitive(true);
        let response = self
            .client
            .post(endpoint)
            .header("authorization", authorization)
            .header("dpop", proof)
            .json(heartbeat)
            .send()
            .await?;
        if !response.status().is_success() {
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) {
                *self.lease.lock().await = None;
            }
            return Err(ApiStatus(response.status()).into());
        }
        let status: DeviceStatus = response_json(response).await?;
        ensure!(
            status.device_id == self.manifest.device_id
                && status.owner_id == self.manifest.owner_id
                && status.name == self.manifest.name
                && status.identity == self.keys.identity()
                && status.status == DeviceRegistrationStatus::Active,
            "Device heartbeat response changed the registered identity"
        );
        Ok(())
    }
}

pub async fn maintain_presence(state_dir: PathBuf, cancel: CancellationToken) -> Result<()> {
    let Some(session) = DeviceSession::load(&state_dir)? else {
        return Ok(());
    };
    let session = Arc::new(session);
    maintain_presence_with_session(state_dir, session, cancel).await
}

pub async fn maintain_presence_with_session(
    state_dir: PathBuf,
    session: Arc<DeviceSession>,
    cancel: CancellationToken,
) -> Result<()> {
    let started = Instant::now();
    let boot_id = crate::host::boot_id()?;
    let mut delay = 2;
    loop {
        let heartbeat = DeviceHeartbeat {
            version: PROTOCOL_VERSION,
            boot_id: boot_id.clone(),
            agent_version: env!("CARGO_PKG_VERSION").into(),
            uptime_seconds: started.elapsed().as_secs(),
        };
        let result = tokio::select! { _ = cancel.cancelled() => return Ok(()), result = session.heartbeat(&heartbeat) => result };
        let mut store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let mut record = store
            .registration()?
            .context("Device registration disappeared")?;
        let wait = match result {
            Ok(()) => {
                record.last_contact_at = Some(unix_time()?);
                record.connection_status = "connected".into();
                delay = 2;
                60
            }
            Err(error) => {
                record.connection_status = if matches!(
                    api_status(&error),
                    Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                ) {
                    "access_denied"
                } else {
                    "disconnected"
                }
                .into();
                tracing::warn!("Device presence unavailable: {error}");
                let wait = delay;
                delay = (delay * 2).min(60);
                wait
            }
        };
        store.update_registration(&record)?;
        if record.connection_status == "access_denied" {
            tracing::warn!(
                "Device access was denied; use recover-enrollment to recheck its registration"
            );
            return Ok(());
        }
        drop(store);
        let jitter = u64::from(OsRng.next_u32() % 10);
        tokio::select! { _ = cancel.cancelled() => return Ok(()), _ = tokio::time::sleep(Duration::from_secs(wait + jitter)) => {} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn missing_binary_fails_before_reserving_an_enrollment() {
        let dir = tempfile::tempdir().unwrap();
        let error = create_package(
            &dir.path().join("controllers"),
            "http://127.0.0.1:9/api/v1",
            "Device",
            &dir.path().join("package"),
            "test-user-token",
            b"password for package tests",
            &dir.path().join("missing-binary"),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Open standalone binary"));
        assert!(!dir.path().join("package").exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn staged_packages_publish_completely_and_never_replace_existing_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("package");
        let mut stage = PackageStage::create(dir.path()).unwrap();
        let staging_path = stage.path.clone();
        vault::write_new_private(&stage.path.join("payload"), b"complete").unwrap();
        stage.publish(&output).unwrap();
        drop(stage);
        assert!(!staging_path.exists());
        assert_eq!(std::fs::read(output.join("payload")).unwrap(), b"complete");

        // An empty destination must also be preserved. Plain rename would replace it.
        let existing = dir.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        let mut other = PackageStage::create(dir.path()).unwrap();
        let other_path = other.path.clone();
        vault::write_new_private(&other.path.join("payload"), b"unpublished").unwrap();
        assert!(other.publish(&existing).is_err());
        drop(other);
        assert!(!other_path.exists());
        assert_eq!(std::fs::read_dir(existing).unwrap().count(), 0);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn parent_sync_failure_preserves_the_published_package() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("package");
        let mut stage = PackageStage::create(dir.path()).unwrap();
        let staging_path = stage.path.clone();
        vault::write_new_private(&stage.path.join("payload"), b"complete").unwrap();
        let error = stage
            .publish_with_parent_sync(&output, |parent| {
                assert_eq!(parent, dir.path());
                assert_eq!(std::fs::read(output.join("payload")).unwrap(), b"complete");
                Err(std::io::Error::other("injected directory sync failure").into())
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Persist published package directory")
        );
        assert!(stage.published);
        drop(stage);
        assert!(!staging_path.exists());
        assert_eq!(std::fs::read(output.join("payload")).unwrap(), b"complete");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn missing_consumed_package_explains_recovery_without_replacing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let state = supervisor::prepare_state_dir(&dir.path().join("state")).unwrap();
        let package = dir.path().join("package");
        std::fs::create_dir(&package).unwrap();
        let keys = DeviceKeys::load_or_create(&state).unwrap();
        let identity = keys.identity();
        let (mut record, _, receipt) = registration_fixture();
        record.receipt = Some(receipt);
        let mut store = StateStore::open(&state.join("management.sqlite")).unwrap();
        store.begin_registration(&record).unwrap();
        let before = serde_json::to_value(store.registration().unwrap()).unwrap();
        drop(store);

        let error = enroll(&state, &package).await.unwrap_err();
        assert!(error.to_string().contains("recover-enrollment"));
        assert!(error.to_string().contains("same --state-dir"));
        assert!(!package.join("onboarding.json").exists());

        // An existing invalid package must still fail rather than using persisted credentials.
        vault::write_new_private(&package.join("onboarding.json"), b"{}").unwrap();
        let error = enroll(&state, &package).await.unwrap_err();
        assert!(!error.to_string().contains("recover-enrollment"));
        assert_eq!(
            std::fs::read(package.join("onboarding.json")).unwrap(),
            b"{}"
        );
        assert_eq!(DeviceKeys::load(&state).unwrap().identity(), identity);
        let store = StateStore::open(&state.join("management.sqlite")).unwrap();
        assert_eq!(
            serde_json::to_value(store.registration().unwrap()).unwrap(),
            before
        );
    }

    #[tokio::test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[ignore = "requires binding a loopback listener"]
    async fn failed_export_cancels_its_reservation_and_preserves_competing_files() {
        use axum::{
            Json, Router,
            extract::State,
            routing::{delete, post},
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct Api {
            base: String,
            output: PathBuf,
            enrollment_id: String,
            device_id: String,
            cancellations: Arc<AtomicUsize>,
            cancellation_status: StatusCode,
        }

        async fn create(
            State(api): State<Api>,
            Json(request): Json<CreateEnrollmentRequest>,
        ) -> Json<CreateEnrollmentResponse> {
            // Simulate another process claiming the output while the request is in flight.
            std::fs::create_dir(&api.output).unwrap();
            std::fs::write(api.output.join("existing-file"), b"preserve me").unwrap();
            let now = unix_time().unwrap();
            Json(CreateEnrollmentResponse {
                enrollment_token: "test-enrollment-token".into(),
                manifest: OnboardingManifest {
                    version: PROTOCOL_VERSION,
                    enrollment_id: api.enrollment_id,
                    device_id: api.device_id,
                    owner_id: "owner".into(),
                    name: request.name,
                    api_base_url: api.base,
                    bootstrap_key: request.bootstrap_key,
                    controller_key: request.controller_key,
                    owner_invitation_key: request.owner_invitation_key,
                    issued_at: now,
                    expires_at: now + 3600,
                },
            })
        }

        async fn cancel(
            State(api): State<Api>,
            axum::extract::Path(id): axum::extract::Path<String>,
        ) -> StatusCode {
            assert_eq!(id, api.enrollment_id);
            api.cancellations.fetch_add(1, Ordering::SeqCst);
            api.cancellation_status
        }

        for cancellation_status in [StatusCode::NO_CONTENT, StatusCode::SERVICE_UNAVAILABLE] {
            let dir = tempfile::tempdir().unwrap();
            let binary = dir.path().join("binary");
            std::fs::write(&binary, b"test executable").unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api = Api {
                base: format!("http://{}/api/v1", listener.local_addr().unwrap()),
                output: dir.path().join("package"),
                enrollment_id: Uuid::new_v4().to_string(),
                device_id: Uuid::new_v4().to_string(),
                cancellations: Arc::new(AtomicUsize::new(0)),
                cancellation_status,
            };
            let app = Router::new()
                .route("/api/v1/devices/enrollments", post(create))
                .route("/api/v1/devices/enrollments/{id}", delete(cancel))
                .with_state(api.clone());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let controllers = dir.path().join("controllers");
            let error = create_package(
                &controllers,
                &api.base,
                "Device",
                &api.output,
                "test-user-token",
                b"password for package tests",
                &binary,
            )
            .await
            .unwrap_err();
            assert_eq!(api.cancellations.load(Ordering::SeqCst), 1);
            assert_eq!(
                std::fs::read(api.output.join("existing-file")).unwrap(),
                b"preserve me"
            );
            assert!(!api.output.join("onboarding.json").exists());
            assert!(
                controllers
                    .join(format!("{}.vault", api.device_id))
                    .is_file()
            );
            assert!(
                controllers
                    .join(format!("{}.invitation.vault", api.device_id))
                    .is_file()
            );
            assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".flow-like-package-")
            }));
            let message = format!("{error:#}");
            assert!(message.contains(&api.enrollment_id));
            assert!(!message.contains("test-user-token"));
            assert!(!message.contains("test-enrollment-token"));
            if cancellation_status == StatusCode::NO_CONTENT {
                assert!(message.contains("was cancelled"));
            } else {
                assert!(message.contains(&format!(
                    "DELETE {}/devices/enrollments/{}",
                    api.base, api.enrollment_id
                )));
            }
            server.abort();
        }
    }

    fn registration_fixture() -> (RegistrationRecord, DeviceIdentity, DeviceReceipt) {
        let controller = SigningKey::generate();
        let bootstrap = SigningKey::generate();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            telemetry_key: SigningKey::generate().public_key(),
        };
        let now = unix_time().unwrap();
        let manifest = OnboardingManifest {
            version: PROTOCOL_VERSION,
            enrollment_id: Uuid::new_v4().to_string(),
            device_id: Uuid::new_v4().to_string(),
            owner_id: "owner".into(),
            name: "Device".into(),
            api_base_url: "https://api.example.test/api/v1".into(),
            bootstrap_key: bootstrap.public_key(),
            controller_key: controller.public_key(),
            owner_invitation_key: SigningKey::generate().public_key(),
            issued_at: now,
            expires_at: now + 3600,
        };
        let manifest_jws = sign_manifest(&manifest, &controller).unwrap();
        let binding = EnrollmentBinding {
            version: PROTOCOL_VERSION,
            enrollment_id: manifest.enrollment_id.clone(),
            device_id: manifest.device_id.clone(),
            identity: identity.clone(),
            manifest_digest: compact_digest(&manifest_jws),
            challenge_id: Uuid::new_v4().to_string(),
            challenge_nonce: Uuid::new_v4().to_string(),
            issued_at: now,
            expires_at: now + 60,
        };
        let binding_jws = sign_binding(&binding, &bootstrap).unwrap();
        let receipt = DeviceReceipt {
            enrollment_id: manifest.enrollment_id.clone(),
            device_id: manifest.device_id.clone(),
            owner_id: manifest.owner_id.clone(),
            name: manifest.name.clone(),
            identity: identity.clone(),
            manifest_jws: manifest_jws.clone(),
            binding_jws: binding_jws.clone(),
            registered_at: now,
            auth_epoch: 1,
        };
        let record = RegistrationRecord {
            manifest,
            manifest_jws,
            binding_jws: Some(binding_jws),
            attempted_bindings: Vec::new(),
            receipt: None,
            last_contact_at: None,
            connection_status: "pending".into(),
        };
        (record, identity, receipt)
    }

    #[test]
    fn device_keys_survive_restart_and_never_replace_missing_registered_keys() {
        let dir = tempfile::tempdir().unwrap();
        let first = DeviceKeys::load_or_create(dir.path()).unwrap().identity();
        assert_eq!(
            first,
            DeviceKeys::load_or_create(dir.path()).unwrap().identity()
        );
        assert_ne!(first.auth_key, first.telemetry_key);
        std::fs::remove_file(dir.path().join("device.keys")).unwrap();
        assert!(DeviceKeys::load(dir.path()).is_err());
    }

    #[test]
    fn uncertain_previous_binding_remains_recoverable_after_a_new_attempt_and_restart() {
        let (mut record, identity, receipt) = registration_fixture();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("management.sqlite");
        let mut store = StateStore::open(&path).unwrap();
        store.begin_registration(&record).unwrap();
        remember_binding(&mut record, "newer.signed.binding".into()).unwrap();
        store.update_registration(&record).unwrap();
        drop(store);
        let restored = StateStore::open(&path)
            .unwrap()
            .registration()
            .unwrap()
            .unwrap();
        assert_eq!(restored.attempted_bindings.len(), 2);
        check_receipt(&restored, &identity, &receipt).unwrap();
        let mut substituted = receipt.clone();
        substituted.binding_jws = "unknown.signed.binding".into();
        assert!(check_receipt(&restored, &identity, &substituted).is_err());
        substituted = receipt;
        substituted.identity.management_key = [43; 32];
        assert!(check_receipt(&restored, &identity, &substituted).is_err());
    }

    #[test]
    fn legacy_binding_is_preserved_and_history_never_discards_old_attempts() {
        let (record, _, _) = registration_fixture();
        let mut legacy = serde_json::to_value(&record).unwrap();
        legacy.as_object_mut().unwrap().remove("attempted_bindings");
        let mut restored: RegistrationRecord = serde_json::from_value(legacy).unwrap();
        assert!(restored.attempted_bindings.is_empty());
        remember_binding(&mut restored, "next.signed.binding".into()).unwrap();
        assert!(
            restored
                .attempted_bindings
                .contains(record.binding_jws.as_ref().unwrap())
        );
        restored.attempted_bindings = (0..MAX_ENROLLMENT_BINDING_ATTEMPTS)
            .map(|index| format!("binding-{index}"))
            .collect();
        restored.binding_jws = restored.attempted_bindings.last().cloned();
        let before = restored.attempted_bindings.clone();
        let previous = restored.binding_jws.clone();
        assert!(ensure_binding_capacity(&restored).is_err());
        assert!(remember_binding(&mut restored, "one-too-many".into()).is_err());
        assert_eq!(restored.attempted_bindings, before);
        assert_eq!(restored.binding_jws, previous);
        restored.attempted_bindings.pop();
        // The missing legacy current binding still occupies the final slot.
        assert!(ensure_binding_capacity(&restored).is_err());
        restored.binding_jws = restored.attempted_bindings.last().cloned();
        assert!(ensure_binding_capacity(&restored).is_ok());
    }
}
