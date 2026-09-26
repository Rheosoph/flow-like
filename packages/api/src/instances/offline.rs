use super::*;
use crate::capacity::PreparedUploadReservation;
use crate::credentials::RuntimeCredentials;
use crate::middleware::jwt::AppUser;
use crate::permission::role_permission::RolePermissions;
use crate::routes::app::invoke::offline_replay::{desktop_access, desktop_forbidden};
use axum::http::StatusCode;
use base64::engine::general_purpose::STANDARD;
use flow_like_storage::{
    Path,
    databases::vector::offline_replay::ReplayMutation,
    files::store::FlowLikeStore,
    object_store::{
        self, GetOptions, GetRange, ObjectStore, ObjectStoreExt, PutMode, PutOptions, UpdateVersion,
    },
};
use sea_orm::{DatabaseTransaction, QueryResult};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[cfg(any(feature = "azure", feature = "gcp"))]
use std::time::Duration;

fn response(
    request: &OfflineReplayRequest,
    digest: &str,
    status: OfflineReplayStatus,
    result: Option<OfflineExpected>,
    message: Option<&str>,
) -> OfflineReplayResponse {
    OfflineReplayResponse {
        operation_id: request.operation_id.clone(),
        digest: digest.into(),
        status,
        result,
        message: message.map(str::to_owned),
    }
}

#[derive(Clone)]
struct ReceiptKey {
    grant_id: String,
    authz_version: i64,
    operation_id: String,
    digest: String,
}

impl ReceiptKey {
    fn new(
        claims: &project::ProjectClaims,
        request: &OfflineReplayRequest,
        digest: &str,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            grant_id: claims.grant_id.clone(),
            authz_version: i64::try_from(claims.authz_version).map_err(|_| ApiError::FORBIDDEN)?,
            operation_id: request.operation_id.clone(),
            digest: digest.into(),
        })
    }
    fn values(&self) -> Vec<sea_orm::Value> {
        vec![
            self.grant_id.clone().into(),
            self.authz_version.into(),
            self.operation_id.clone().into(),
        ]
    }
}

pub(crate) struct DesktopReplayPrincipal {
    pub(crate) sub: String,
    pub(crate) app_id: String,
    pub(crate) installation_id: String,
}

impl DesktopReplayPrincipal {
    /// Instance grant IDs are identifiers without ':', so desktop keys never collide with them.
    fn grant_id(&self) -> Result<String, ApiError> {
        let scope = serde_json::to_vec(&(
            "flow-like-desktop-offline-v1",
            &self.sub,
            &self.app_id,
            &self.installation_id,
        ))?;
        Ok(format!(
            "desktop:{}",
            flow_like_storage::blake3::hash(&scope).to_hex()
        ))
    }
}

enum DesktopAuthority<'a> {
    Session {
        state: &'a AppState,
        user: &'a AppUser,
    },
    #[cfg(test)]
    Fixed(RolePermissions),
}

enum ReplayPrincipal<'a> {
    Instance(project::AuthorizedProject),
    Desktop {
        principal: DesktopReplayPrincipal,
        authority: DesktopAuthority<'a>,
    },
}

#[derive(Clone)]
struct ReceiptOwner {
    instance_id: String,
    project_id: String,
    sub: String,
}

impl ReplayPrincipal<'_> {
    fn project_id(&self) -> &str {
        match self {
            Self::Instance(authorization) => &authorization.claims.project_id,
            Self::Desktop { principal, .. } => &principal.app_id,
        }
    }

    fn sub(&self) -> &str {
        match self {
            Self::Instance(authorization) => &authorization.claims.sub,
            Self::Desktop { principal, .. } => &principal.sub,
        }
    }

    fn instance_column(&self) -> &str {
        match self {
            Self::Instance(authorization) => &authorization.claims.instance_id,
            Self::Desktop { principal, .. } => &principal.installation_id,
        }
    }

    fn owner(&self) -> ReceiptOwner {
        ReceiptOwner {
            instance_id: self.instance_column().into(),
            project_id: self.project_id().into(),
            sub: self.sub().into(),
        }
    }

    fn receipt_key(
        &self,
        request: &OfflineReplayRequest,
        digest: &str,
    ) -> Result<ReceiptKey, ApiError> {
        match self {
            Self::Instance(authorization) => {
                ReceiptKey::new(&authorization.claims, request, digest)
            }
            Self::Desktop { principal, .. } => Ok(ReceiptKey {
                grant_id: principal.grant_id()?,
                authz_version: 0,
                operation_id: request.operation_id.clone(),
                digest: digest.into(),
            }),
        }
    }

    /// The grant row an instance claim locks; desktop claims serialize on the receipt key.
    fn claim_lock(&self) -> Option<project::AuthorizedProject> {
        match self {
            Self::Instance(authorization) => Some(authorization.clone()),
            Self::Desktop { .. } => None,
        }
    }

    async fn recheck(
        &self,
        context: &DeviceContext<'_>,
        resource: &OfflineResource,
    ) -> Result<(), ApiError> {
        let (principal, authority) = match self {
            Self::Instance(authorization) => {
                return project::recheck(context, authorization).await.map(|_| ());
            }
            Self::Desktop {
                principal,
                authority,
            } => (principal, authority),
        };
        let permissions: RolePermissions = match authority {
            DesktopAuthority::Session { state, user } => {
                user.app_permission_fresh(&principal.app_id, state)
                    .await?
                    .permissions
            }
            #[cfg(test)]
            DesktopAuthority::Fixed(permissions) => *permissions,
        };
        if desktop_access(&permissions, resource) != Some(true) {
            return Err(desktop_forbidden());
        }
        Ok(())
    }

    async fn upload_deadline(&self, context: &DeviceContext<'_>) -> Result<i64, ApiError> {
        Ok(match self {
            Self::Instance(authorization) => project::recheck_storage(context, authorization)
                .await?
                .min(now() + 300),
            Self::Desktop { .. } => now() + 300,
        })
    }

    async fn record_applied(&self, request: &OfflineReplayRequest) {
        let Self::Desktop {
            principal,
            authority: DesktopAuthority::Session { state, user },
        } = self
        else {
            return;
        };
        let (action, resource_type, resource_id, details) = desktop_audit(request);
        crate::audit_branch!(
            state,
            user,
            principal.app_id,
            action,
            resource_type,
            resource_id,
            details
        );
    }
}

fn ensure_same_digest(retained: &str, key: &ReceiptKey) -> Result<(), ApiError> {
    if retained != key.digest {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            OFFLINE_ERROR_DIGEST_REUSED,
            "Offline operation ID was already used with another payload or precondition",
        ));
    }
    Ok(())
}

fn decode_receipt(
    row: QueryResult,
    key: &ReceiptKey,
) -> Result<Option<OfflineReplayResponse>, ApiError> {
    ensure_same_digest(&row.try_get::<String>("", "digest")?, key)?;
    row.try_get::<Option<String>>("", "result")?
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|_| ApiError::internal("Invalid retained offline receipt"))
        })
        .transpose()
}

async fn receipt<C: ConnectionTrait>(
    db: &C,
    key: &ReceiptKey,
) -> Result<Option<Option<OfflineReplayResponse>>, ApiError> {
    db.query_one_raw(sql(r#"SELECT digest,result FROM "InstanceOfflineReceipt" WHERE "grantId"=$1 AND "authzVersion"=$2 AND "operationId"=$3"#,key.values())).await?.map(|row| decode_receipt(row,key)).transpose()
}

fn attempt_values(key: &ReceiptKey, owner: &ReceiptOwner) -> Vec<sea_orm::Value> {
    vec![
        key.grant_id.clone().into(),
        key.authz_version.into(),
        key.operation_id.clone().into(),
        key.digest.clone().into(),
        owner.instance_id.clone().into(),
        owner.project_id.clone().into(),
        owner.sub.clone().into(),
        now().into(),
    ]
}

async fn insert_attempt(
    tx: &DatabaseTransaction,
    key: &ReceiptKey,
    owner: &ReceiptOwner,
) -> Result<(), ApiError> {
    tx.execute_raw(sql(r#"INSERT INTO "InstanceOfflineReceipt" ("grantId","authzVersion","operationId",digest,"instanceId","projectId","delegatingUserId",status,"createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,'attempting',$8,$8)"#,
        attempt_values(key, owner))).await?;
    Ok(())
}

async fn reopen_blocked(tx: &DatabaseTransaction, key: &ReceiptKey) -> Result<u64, ApiError> {
    let mut values = key.values();
    values.extend([key.digest.clone().into(), now().into()]);
    Ok(tx.execute_raw(sql(r#"UPDATE "InstanceOfflineReceipt" SET result=NULL,status='attempting',"updatedAt"=$5 WHERE "grantId"=$1 AND "authzVersion"=$2 AND "operationId"=$3 AND digest=$4 AND status='blocked'"#, values)).await?.rows_affected())
}

fn is_blocked(retained: &Option<Option<OfflineReplayResponse>>) -> bool {
    matches!(retained, Some(Some(value)) if value.status == OfflineReplayStatus::Blocked)
}

async fn claim_instance(
    tx: &DatabaseTransaction,
    authorization: &project::AuthorizedProject,
    key: &ReceiptKey,
    owner: &ReceiptOwner,
    reservation: Option<PreparedUploadReservation>,
) -> Result<bool, ApiError> {
    // lock_graph writes the retained grant row, serializing claims across
    // API replicas and workload instances sharing this placement grant.
    project::recheck_in(tx, authorization).await?;
    let retained = receipt(tx, key).await?;
    let retry_blocked = is_blocked(&retained);
    if retained.is_some() && !retry_blocked {
        return Ok(false);
    }
    if let Some(reservation) = reservation {
        reservation.reserve(tx).await?;
    }
    if retry_blocked {
        // Only definitive no-effect failures are retained as Blocked.
        // The same immutable request may retry after fresh admission;
        // an uncertain provider outcome never reaches this branch.
        reopen_blocked(tx, key).await?;
    } else {
        insert_attempt(tx, key, owner).await?;
    }
    Ok(true)
}

/// Without a grant row to lock, the receipt primary key decides which request dispatches.
async fn claim_desktop(
    tx: &DatabaseTransaction,
    key: &ReceiptKey,
    owner: &ReceiptOwner,
    reservation: Option<PreparedUploadReservation>,
) -> Result<bool, ApiError> {
    let retained = receipt(tx, key).await?;
    let retry_blocked = is_blocked(&retained);
    if retained.is_some() && !retry_blocked {
        return Ok(false);
    }
    let claimed = if retry_blocked {
        reopen_blocked(tx, key).await? == 1
    } else {
        tx.query_one_raw(sql(r#"INSERT INTO "InstanceOfflineReceipt" ("grantId","authzVersion","operationId",digest,"instanceId","projectId","delegatingUserId",status,"createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,'attempting',$8,$8) ON CONFLICT ("grantId","authzVersion","operationId") DO NOTHING RETURNING "operationId""#,
            attempt_values(key, owner))).await?.is_some()
    };
    if !claimed {
        return Ok(false);
    }
    if let Some(reservation) = reservation {
        reservation.reserve(tx).await?;
    }
    Ok(true)
}

async fn claim(
    state: &DeviceContext<'_>,
    principal: &ReplayPrincipal<'_>,
    key: &ReceiptKey,
    reservation: Option<PreparedUploadReservation>,
) -> Result<bool, ApiError> {
    let lock = principal.claim_lock();
    let owner = principal.owner();
    let key = key.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let lock = lock.clone();
            let owner = owner.clone();
            let key = key.clone();
            let reservation = reservation.clone();
            Box::pin(async move {
                match lock {
                    Some(authorization) => {
                        claim_instance(tx, &authorization, &key, &owner, reservation).await
                    }
                    None => claim_desktop(tx, &key, &owner, reservation).await,
                }
            })
        },
    )
    .await
}

async fn finish(
    state: &DeviceContext<'_>,
    key: &ReceiptKey,
    value: OfflineReplayResponse,
) -> Result<OfflineReplayResponse, ApiError> {
    retain(state, key, value).await.map(|(value, _)| value)
}

/// Retains a dispatched outcome; each receipt records its Applied audit event exactly once,
/// from the request whose write retained it.
async fn finish_dispatched(
    state: &DeviceContext<'_>,
    principal: &ReplayPrincipal<'_>,
    key: &ReceiptKey,
    request: &OfflineReplayRequest,
    value: OfflineReplayResponse,
) -> Result<OfflineReplayResponse, ApiError> {
    let (value, retained_here) = retain(state, key, value).await?;
    if retained_here && value.status == OfflineReplayStatus::Applied {
        principal.record_applied(request).await;
    }
    Ok(value)
}

/// The retained response, and whether this call's write retained it.
async fn retain(
    state: &DeviceContext<'_>,
    key: &ReceiptKey,
    value: OfflineReplayResponse,
) -> Result<(OfflineReplayResponse, bool), ApiError> {
    // Unknown responses stay reconcilable. They are never changed back to a
    // dispatchable state, including after a process crash or request timeout.
    if value.status == OfflineReplayStatus::OutcomeUnknown {
        return Ok((value, false));
    }
    let mut values = key.values();
    let status = match value.status {
        OfflineReplayStatus::Applied => "applied",
        OfflineReplayStatus::Conflict => "conflict",
        OfflineReplayStatus::Unsupported => "unsupported",
        OfflineReplayStatus::Blocked => "blocked",
        OfflineReplayStatus::OutcomeUnknown => unreachable!(),
    };
    values.extend([
        serde_json::to_string(&value)?.into(),
        status.into(),
        now().into(),
        key.digest.clone().into(),
    ]);
    // A fresh retry may claim a definitive Blocked result immediately. Return
    // the value from this write before that retry can clear the retained result.
    if let Some(row) = state.db.query_one_raw(sql(r#"UPDATE "InstanceOfflineReceipt" SET result=$4,status=$5,"updatedAt"=$6 WHERE "grantId"=$1 AND "authzVersion"=$2 AND "operationId"=$3 AND digest=$7 AND result IS NULL RETURNING digest,result"#,values)).await? {
        return decode_receipt(row, key)?
            .map(|value| (value, true))
            .ok_or_else(|| ApiError::internal("Offline receipt result was not retained"));
    }
    receipt(state.db, key)
        .await?
        .flatten()
        .map(|value| (value, false))
        .ok_or_else(|| ApiError::internal("Offline receipt disappeared"))
}

fn unknown(request: &OfflineReplayRequest, digest: &str) -> OfflineReplayResponse {
    response(
        request,
        digest,
        OfflineReplayStatus::OutcomeUnknown,
        None,
        Some(
            "The provider outcome is not proven. The operation has not been dispatched again; inspect or explicitly skip it.",
        ),
    )
}

fn content_credentials(master: &RuntimeCredentials) -> &RuntimeCredentials {
    let mut credentials = master;
    while let RuntimeCredentials::Mixed(mixed) = credentials {
        credentials = &mixed.content;
    }
    credentials
}

fn content_bucket(credentials: &RuntimeCredentials) -> &str {
    match content_credentials(credentials) {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(c) => &c.content_bucket,
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(c) => &c.content_bucket,
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(c) => &c.content_container,
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(c) => &c.content_bucket,
        RuntimeCredentials::Mixed(_) => unreachable!(),
    }
}

pub(crate) fn content_provider(credentials: &RuntimeCredentials) -> OfflineContentProvider {
    match content_credentials(credentials) {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(_) => OfflineContentProvider::S3,
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(_) => OfflineContentProvider::S3,
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(_) => OfflineContentProvider::Az,
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(_) => OfflineContentProvider::Gs,
        RuntimeCredentials::Mixed(_) => unreachable!(),
    }
}

async fn file_store(credentials: &RuntimeCredentials) -> Result<FlowLikeStore, ApiError> {
    let shared = content_credentials(credentials).into_shared_credentials();
    flow_like_types::tokio::task::spawn_blocking(
        move || -> flow_like_types::Result<FlowLikeStore> {
            use flow_like::credentials::SharedCredentials;
            // Even conditional requests are retried after some 5xx responses by
            // object_store. A durable replay attempt must send its mutation once.
            let retry = object_store::RetryConfig {
                max_retries: 0,
                ..Default::default()
            };
            match shared {
                SharedCredentials::Aws(c) => {
                    let mut builder = object_store::aws::AmazonS3Builder::new()
                        .with_bucket_name(c.content_bucket)
                        .with_region(c.region)
                        .with_retry(retry);
                    if let Some(value) = c.access_key_id {
                        builder = builder.with_access_key_id(value);
                    }
                    if let Some(value) = c.secret_access_key {
                        builder = builder.with_secret_access_key(value);
                    }
                    if let Some(value) = c.session_token {
                        builder = builder.with_token(value);
                    }
                    if c.content_config
                        .as_ref()
                        .is_some_and(|config| config.express)
                    {
                        return Err(flow_like_types::anyhow!(
                            "Offline replay does not support S3 directory buckets"
                        ));
                    }
                    for (key, value) in flow_like::credentials::aws_credentials::s3_storage_options(
                        c.content_config.as_ref(),
                    ) {
                        if key == "allow_http" {
                            builder = builder.with_allow_http(value == "true");
                        } else {
                            builder = builder.with_config(key.parse()?, value);
                        }
                    }
                    Ok(FlowLikeStore::AWS(Arc::new(builder.build()?)))
                }
                SharedCredentials::Azure(c) => {
                    let mut builder = object_store::azure::MicrosoftAzureBuilder::new()
                        .with_account(c.account_name)
                        .with_container_name(c.content_container)
                        .with_retry(retry);
                    if let Some(key) = c.account_key {
                        builder = builder.with_access_key(key);
                    } else if let Some(sas) = c.content_sas_token {
                        let url = reqwest::Url::parse(&format!(
                            "https://scope.invalid/?{}",
                            sas.trim_start_matches('?')
                        ))?;
                        builder = builder.with_sas_authorization(
                            url.query_pairs()
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect::<Vec<_>>(),
                        );
                    }
                    Ok(FlowLikeStore::Azure(Arc::new(builder.build()?)))
                }
                SharedCredentials::Gcp(c) => {
                    let mut builder = object_store::gcp::GoogleCloudStorageBuilder::new()
                        .with_bucket_name(c.content_bucket)
                        .with_retry(retry);
                    if let Some(token) = c.access_token {
                        builder = builder.with_credentials(Arc::new(
                            object_store::StaticCredentialProvider::new(
                                object_store::gcp::GcpCredential { bearer: token },
                            ),
                        ));
                    } else if !c.service_account_key.is_empty() {
                        builder = builder.with_service_account_key(c.service_account_key);
                    }
                    Ok(FlowLikeStore::Google(Arc::new(builder.build()?)))
                }
                SharedCredentials::Renewable(_) => Err(flow_like_types::anyhow!(
                    "Offline replay requires API-issued provider credentials"
                )),
                SharedCredentials::Mixed(_) => unreachable!("content credentials are unwrapped"),
            }
        },
    )
    .await
    .map_err(|_| ApiError::service_unavailable("Cannot initialize offline file storage"))?
    .map_err(|_| ApiError::service_unavailable("Cannot initialize offline file storage"))
}

fn file_path(
    sub: &str,
    project: &str,
    purpose: StoragePurpose,
    relative: &str,
) -> Result<Path, ApiError> {
    validate_offline_relative_path(relative).map_err(invalid)?;
    let prefix = crate::credentials::device_execute_prefixes(sub, project)
        .map_err(|_| ApiError::FORBIDDEN)?
        .remove(&purpose)
        .ok_or(ApiError::FORBIDDEN)?;
    // Prefixes already carry the server's object-key encoding (including Auth0
    // subjects). Parsing preserves it; Path::from would encode it a second time.
    let path = Path::parse(format!("{prefix}{relative}"))
        .map_err(|_| ApiError::bad_request("Invalid offline file path"))?;
    if !path.as_ref().starts_with(&prefix) {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(path)
}

fn file_support(
    credentials: &RuntimeCredentials,
    expected: &OfflineExpected,
    mutation: &OfflineMutation,
) -> Option<&'static str> {
    if matches!(expected, OfflineExpected::FileAbsent) {
        return None;
    }
    match (content_credentials(credentials), expected) {
        #[cfg(feature = "azure")]
        (RuntimeCredentials::Azure(_), OfflineExpected::FileRevision { e_tag: Some(_), .. }) => {
            None
        }
        #[cfg(feature = "gcp")]
        (
            RuntimeCredentials::Gcp(_),
            OfflineExpected::FileRevision {
                version: Some(v), ..
            },
        ) if v.parse::<u64>().is_ok_and(|v| v > 0) => None,
        _ => Some(if matches!(mutation, OfflineMutation::FileDelete) {
            "This provider cannot conditionally delete an exact object revision; offline deletion is unsupported"
        } else {
            "This provider cannot fence an exact object revision against replacement; offline overwrite is unsupported"
        }),
    }
}

fn provider_failure(
    request: &OfflineReplayRequest,
    digest: &str,
    error: &object_store::Error,
) -> OfflineReplayResponse {
    match error {
        object_store::Error::AlreadyExists { .. }
        | object_store::Error::Precondition { .. }
        | object_store::Error::NotFound { .. } => response(
            request,
            digest,
            OfflineReplayStatus::Conflict,
            None,
            Some("The cloud object no longer matches the queued precondition"),
        ),
        object_store::Error::NotSupported { .. } | object_store::Error::NotImplemented { .. } => {
            response(
                request,
                digest,
                OfflineReplayStatus::Unsupported,
                None,
                Some("The cloud provider does not support the required conditional operation"),
            )
        }
        object_store::Error::PermissionDenied { .. }
        | object_store::Error::Unauthenticated { .. } => response(
            request,
            digest,
            OfflineReplayStatus::Blocked,
            None,
            Some("The cloud provider denied the operation"),
        ),
        _ => unknown(request, digest),
    }
}

/// The object's revision when it holds exactly `len` bytes with this SHA-256.
async fn stored_revision(
    store: &dyn ObjectStore,
    path: &Path,
    len: usize,
    sha256: &str,
) -> Option<OfflineExpected> {
    let options = GetOptions {
        range: (len > 0).then(|| GetRange::Bounded(0..len as u64 + 1)),
        ..Default::default()
    };
    let result = store.get_opts(path, options).await.ok()?;
    if result.meta.size != len as u64 {
        return None;
    }
    let revision = OfflineExpected::FileRevision {
        e_tag: result.meta.e_tag.clone(),
        version: result.meta.version.clone(),
    };
    let bytes = result.bytes().await.ok()?;
    (bytes.len() == len && format!("{:x}", Sha256::digest(&bytes)) == sha256).then_some(revision)
}

/// An earlier attempt whose outcome was lost is proven applied when the object holds its bytes.
async fn reconcile_put(
    store: &dyn ObjectStore,
    path: &Path,
    request: &OfflineReplayRequest,
    digest: &str,
) -> Option<OfflineReplayResponse> {
    let OfflineMutation::FilePut {
        data_base64,
        sha256,
    } = &request.mutation
    else {
        return None;
    };
    let len = STANDARD.decode(data_base64).ok()?.len();
    let revision = stored_revision(store, path, len, sha256).await?;
    Some(response(
        request,
        digest,
        OfflineReplayStatus::Applied,
        Some(revision),
        None,
    ))
}

async fn put_file(
    store: &dyn ObjectStore,
    path: &Path,
    request: &OfflineReplayRequest,
    digest: &str,
) -> OfflineReplayResponse {
    let OfflineMutation::FilePut {
        data_base64,
        sha256,
    } = &request.mutation
    else {
        return unknown(request, digest);
    };
    let mode = match &request.expected {
        OfflineExpected::FileAbsent => PutMode::Create,
        OfflineExpected::FileRevision { e_tag, version } => PutMode::Update(UpdateVersion {
            e_tag: e_tag.clone(),
            version: version.clone(),
        }),
        _ => return unknown(request, digest),
    };
    let Ok(payload) = STANDARD.decode(data_base64) else {
        return unknown(request, digest);
    };
    let len = payload.len();
    // No automatic retry of a possibly-sent operation is performed by replay.
    // Conditional PUT keeps a concurrent writer from being overwritten.
    match store
        .put_opts(
            path,
            payload.into(),
            PutOptions {
                mode,
                ..Default::default()
            },
        )
        .await
    {
        Ok(result) => response(
            request,
            digest,
            OfflineReplayStatus::Applied,
            Some(OfflineExpected::FileRevision {
                e_tag: result.e_tag,
                version: result.version,
            }),
            None,
        ),
        Err(
            error @ (object_store::Error::AlreadyExists { .. }
            | object_store::Error::Precondition { .. }),
        ) if matches!(request.expected, OfflineExpected::FileAbsent) => {
            match stored_revision(store, path, len, sha256).await {
                Some(revision) => response(
                    request,
                    digest,
                    OfflineReplayStatus::Applied,
                    Some(revision),
                    None,
                ),
                None => provider_failure(request, digest, &error),
            }
        }
        Err(error) => provider_failure(request, digest, &error),
    }
}

#[allow(unused_variables)]
async fn delete_file(
    credentials: &RuntimeCredentials,
    store: &FlowLikeStore,
    path: &Path,
    request: &OfflineReplayRequest,
    digest: &str,
) -> OfflineReplayResponse {
    if matches!(request.expected, OfflineExpected::FileAbsent) {
        return match store.as_generic().head(path).await {
            Err(object_store::Error::NotFound { .. }) => response(
                request,
                digest,
                OfflineReplayStatus::Applied,
                Some(OfflineExpected::FileAbsent),
                None,
            ),
            Ok(_) => response(
                request,
                digest,
                OfflineReplayStatus::Conflict,
                None,
                Some("The queued absent object now exists"),
            ),
            Err(error) => provider_failure(request, digest, &error),
        };
    }
    let OfflineExpected::FileRevision { e_tag, version } = &request.expected else {
        return unknown(request, digest);
    };
    #[cfg(not(any(feature = "azure", feature = "gcp")))]
    return response(
        request,
        digest,
        OfflineReplayStatus::Unsupported,
        None,
        Some("Conditional deletion is not supported for this provider"),
    );
    #[cfg(any(feature = "azure", feature = "gcp"))]
    {
        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(client) => client,
            Err(_) => return unknown(request, digest),
        };
        let call = match content_credentials(credentials) {
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(c) => {
                let Some(e_tag) = e_tag else {
                    return unknown(request, digest);
                };
                let Ok(url) = store.sign("DELETE", path, Duration::from_secs(60)).await else {
                    return response(
                        request,
                        digest,
                        OfflineReplayStatus::Blocked,
                        None,
                        Some("Cannot authorize conditional blob deletion"),
                    );
                };
                if url.scheme() != "https"
                    || url.host_str()
                        != Some(format!("{}.blob.core.windows.net", c.account_name).as_str())
                {
                    return response(
                        request,
                        digest,
                        OfflineReplayStatus::Unsupported,
                        None,
                        Some("Conditional deletion requires the standard Azure Blob endpoint"),
                    );
                }
                client
                    .delete(url)
                    .header(reqwest::header::IF_MATCH, e_tag)
                    .header("x-ms-version", "2023-11-03")
            }
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(c) => {
                let Some(version) = version else {
                    return unknown(request, digest);
                };
                let token = if let Some(token) = &c.access_token {
                    Ok(token.clone())
                } else if let Some(key) = &c.service_account_key {
                    crate::credentials::gcp_credentials::generate_access_token_standalone(
                        key,
                        "https://www.googleapis.com/auth/devstorage.full_control",
                    )
                    .await
                } else {
                    crate::credentials::gcp_credentials::fetch_metadata_token().await
                };
                let Ok(token) = token else {
                    return response(
                        request,
                        digest,
                        OfflineReplayStatus::Blocked,
                        None,
                        Some("Cannot authorize conditional object deletion"),
                    );
                };
                let Ok(mut url) =
                    reqwest::Url::parse("https://storage.googleapis.com/storage/v1/b/")
                else {
                    return unknown(request, digest);
                };
                url.path_segments_mut()
                    .expect("fixed HTTPS base")
                    .pop_if_empty()
                    .push(&c.content_bucket)
                    .push("o")
                    .push(path.as_ref());
                url.query_pairs_mut()
                    .append_pair("ifGenerationMatch", version);
                client.delete(url).bearer_auth(token)
            }
            _ => {
                return response(
                    request,
                    digest,
                    OfflineReplayStatus::Unsupported,
                    None,
                    Some("Conditional deletion is not supported for this provider"),
                );
            }
        };
        match call.send().await {
            Ok(result) if result.status().is_success() => response(
                request,
                digest,
                OfflineReplayStatus::Applied,
                Some(OfflineExpected::FileAbsent),
                None,
            ),
            Ok(result) if matches!(result.status().as_u16(), 404 | 409 | 412) => response(
                request,
                digest,
                OfflineReplayStatus::Conflict,
                None,
                Some("The cloud object no longer matches the queued revision"),
            ),
            Ok(result) if matches!(result.status().as_u16(), 401 | 403) => response(
                request,
                digest,
                OfflineReplayStatus::Blocked,
                None,
                Some("The cloud provider denied conditional deletion"),
            ),
            _ => unknown(request, digest),
        }
    }
}

pub(super) async fn authorize(
    context: &DeviceContext<'_>,
    headers: &HeaderMap,
) -> Result<project::AuthorizedProject, ApiError> {
    let authorization =
        project::authenticate(context, headers, "POST", OFFLINE_REPLAY_PATH).await?;
    if authorization.claims.access != OnlineProjectAccess::ReadWrite
        || authorization.claims.purpose != InstancePurpose::Workload
    {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(authorization)
}

pub(crate) async fn replay(
    state: &AppState,
    headers: &HeaderMap,
    request: OfflineReplayRequest,
) -> Result<OfflineReplayResponse, ApiError> {
    let context = devices::context(state);
    let authorization = authorize(&context, headers).await?;
    request.validate().map_err(invalid)?;
    replay_as(
        state,
        &context,
        ReplayPrincipal::Instance(authorization),
        request,
    )
    .await
}

/// The caller has authenticated the account and applied the desktop permission matrix and limits.
pub(crate) async fn replay_desktop(
    state: &AppState,
    user: &AppUser,
    principal: DesktopReplayPrincipal,
    request: OfflineReplayRequest,
) -> Result<OfflineReplayResponse, ApiError> {
    let context = devices::context(state);
    replay_as(
        state,
        &context,
        ReplayPrincipal::Desktop {
            principal,
            authority: DesktopAuthority::Session { state, user },
        },
        request,
    )
    .await
}

fn desktop_audit(
    request: &OfflineReplayRequest,
) -> (&'static str, &'static str, String, serde_json::Value) {
    let kind = match &request.mutation {
        OfflineMutation::TableInsert { .. } => "table_insert",
        OfflineMutation::TableUpsert { .. } => "table_upsert",
        OfflineMutation::TableUpdate { .. } => "table_update",
        OfflineMutation::TableDelete { .. } => "table_delete",
        OfflineMutation::FilePut { .. } => "file_put",
        OfflineMutation::FileDelete => "file_delete",
    };
    let (action, resource_type, resource_id, purpose) = match &request.resource {
        OfflineResource::Table { purpose, table, .. } => (
            "database.rows.offline_replay",
            "DatabaseTable",
            table.clone(),
            purpose,
        ),
        OfflineResource::File { purpose, path } => (
            "storage.files.offline_replay",
            "StorageFile",
            path.clone(),
            purpose,
        ),
    };
    (
        action,
        resource_type,
        resource_id,
        serde_json::json!({
            "operation_id": request.operation_id,
            "kind": kind,
            "user_scoped": *purpose == StoragePurpose::User,
        }),
    )
}

enum ReplayTarget {
    Table(ReplayMutation),
    File(Path),
}

/// Every request-derived rejection happens here, before a receipt is read or claimed.
fn replay_target(
    principal: &ReplayPrincipal<'_>,
    request: &OfflineReplayRequest,
) -> Result<ReplayTarget, ApiError> {
    match &request.resource {
        OfflineResource::Table { .. } => table_mutation(request).map(ReplayTarget::Table),
        OfflineResource::File { purpose, path } => {
            file_path(principal.sub(), principal.project_id(), *purpose, path)
                .map(ReplayTarget::File)
        }
    }
}

async fn replay_as(
    state: &AppState,
    context: &DeviceContext<'_>,
    principal: ReplayPrincipal<'_>,
    request: OfflineReplayRequest,
) -> Result<OfflineReplayResponse, ApiError> {
    let digest = request.digest().map_err(invalid)?;
    let key = principal.receipt_key(&request, &digest)?;
    let target = replay_target(&principal, &request)?;
    let previous = receipt(context.db, &key).await?;
    if let Some(Some(value)) = previous.as_ref()
        && value.status != OfflineReplayStatus::Blocked
    {
        return Ok(value.clone());
    }
    let previous_attempt = matches!(previous, Some(None));
    let master = state.master_credentials().await.map_err(|_| {
        ApiError::service_unavailable("Offline replay storage credentials are unavailable")
    })?;
    let path = match target {
        ReplayTarget::Table(mutation) => {
            if let OfflineResource::Table { database, .. } = &request.resource
                && database != "db"
            {
                return Ok(response(
                    &request,
                    &digest,
                    OfflineReplayStatus::Unsupported,
                    None,
                    Some("Only the registered project and user database roots are supported"),
                ));
            }
            return replay_table(
                state,
                context,
                &principal,
                &key,
                master.as_ref(),
                mutation,
                request,
                digest,
                previous_attempt,
            )
            .await;
        }
        ReplayTarget::File(path) => path,
    };
    if previous_attempt {
        let reconciled = match &request.mutation {
            OfflineMutation::FilePut { .. } => match file_store(master.as_ref()).await {
                Ok(store) => {
                    reconcile_put(store.as_generic().as_ref(), &path, &request, &digest).await
                }
                Err(_) => None,
            },
            _ => None,
        };
        return match reconciled {
            Some(value) => finish_dispatched(context, &principal, &key, &request, value).await,
            None => Ok(unknown(&request, &digest)),
        };
    }
    if let Some(reason) = file_support(master.as_ref(), &request.expected, &request.mutation) {
        return Ok(response(
            &request,
            &digest,
            OfflineReplayStatus::Unsupported,
            None,
            Some(reason),
        ));
    }
    let store = file_store(master.as_ref()).await?;
    let reservation = if let OfflineMutation::FilePut { data_base64, .. } = &request.mutation {
        let size = STANDARD.decode(data_base64).map_err(invalid)?.len() as u64;
        let expiry = chrono::DateTime::from_timestamp(principal.upload_deadline(context).await?, 0)
            .ok_or(ApiError::UNAUTHORIZED)?
            .fixed_offset();
        match crate::capacity::prepare_upload_reservation(
            state,
            principal.project_id(),
            principal.sub(),
            vec![(
                content_bucket(master.as_ref()).into(),
                path.to_string(),
                size,
            )],
            expiry,
        )
        .await
        {
            Ok(reservation) => Some(reservation),
            Err(error) => {
                return Ok(response(
                    &request,
                    &digest,
                    OfflineReplayStatus::Blocked,
                    None,
                    Some(
                        error
                            .public_message()
                            .unwrap_or("Storage quota admission is unavailable"),
                    ),
                ));
            }
        }
    } else {
        None
    };
    run_file_attempt(
        context,
        &principal,
        &key,
        &path,
        &store,
        master.as_ref(),
        &request,
        &digest,
        reservation,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_file_attempt(
    context: &DeviceContext<'_>,
    principal: &ReplayPrincipal<'_>,
    key: &ReceiptKey,
    path: &Path,
    store: &FlowLikeStore,
    credentials: &RuntimeCredentials,
    request: &OfflineReplayRequest,
    digest: &str,
    reservation: Option<PreparedUploadReservation>,
) -> Result<OfflineReplayResponse, ApiError> {
    match claim(context, principal, key, reservation).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(receipt(context.db, key)
                .await?
                .flatten()
                .unwrap_or_else(|| unknown(request, digest)));
        }
        Err(error) if error.status().as_u16() == 429 || error.status().as_u16() == 402 => {
            return Ok(response(
                request,
                digest,
                OfflineReplayStatus::Blocked,
                None,
                Some(error.public_message().unwrap_or("Storage quota exceeded")),
            ));
        }
        Err(error) => return Err(error),
    }
    // A revocation after admission must stop a request that has not yet reached
    // the provider. A provider outcome is always retained before replying.
    if let Err(error) = principal.recheck(context, &request.resource).await {
        return finish(
            context,
            key,
            response(
                request,
                digest,
                OfflineReplayStatus::Blocked,
                None,
                Some(
                    error
                        .public_message()
                        .unwrap_or("Project authorization changed before replay"),
                ),
            ),
        )
        .await;
    }
    let value = match &request.mutation {
        OfflineMutation::FilePut { .. } => {
            put_file(store.as_generic().as_ref(), path, request, digest).await
        }
        OfflineMutation::FileDelete => delete_file(credentials, store, path, request, digest).await,
        _ => unreachable!(),
    };
    finish_dispatched(context, principal, key, request, value).await
}

#[cfg(test)]
pub(super) async fn assert_receipt_lifecycle(
    context: &DeviceContext<'_>,
    authorization: &project::AuthorizedProject,
) {
    let principal = ReplayPrincipal::Instance(authorization.clone());
    let request = tests::file_request(b"one");
    let digest = request.digest().unwrap();
    let key = ReceiptKey::new(&authorization.claims, &request, &digest).unwrap();
    let (first, second) = flow_like_types::tokio::join!(
        claim(context, &principal, &key, None),
        claim(context, &principal, &key, None)
    );
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "only one API replica may dispatch"
    );
    assert_eq!(receipt(context.db, &key).await.unwrap(), Some(None));
    assert!(
        !claim(context, &principal, &key, None).await.unwrap(),
        "a retained attempt is never redispatched"
    );
    let mut different = key.clone();
    different.digest = "different-payload".into();
    assert_eq!(
        claim(context, &principal, &different, None)
            .await
            .unwrap_err()
            .status()
            .as_u16(),
        409
    );
    let mut other_scope = key.clone();
    other_scope.authz_version += 1;
    assert!(receipt(context.db, &other_scope).await.unwrap().is_none());
    let value = response(
        &request,
        &digest,
        OfflineReplayStatus::Applied,
        Some(OfflineExpected::FileRevision {
            e_tag: Some("cloud-etag".into()),
            version: Some("1".into()),
        }),
        None,
    );
    assert_eq!(finish(context, &key, value.clone()).await.unwrap(), value);
    assert_eq!(
        receipt(context.db, &key).await.unwrap(),
        Some(Some(value.clone()))
    );
    assert_eq!(
        finish(
            context,
            &key,
            response(&request, &digest, OfflineReplayStatus::Conflict, None, None)
        )
        .await
        .unwrap(),
        value,
        "a later request cannot replace a retained result"
    );

    for status in [
        OfflineReplayStatus::Conflict,
        OfflineReplayStatus::Unsupported,
    ] {
        let request = tests::file_request(b"terminal");
        let digest = request.digest().unwrap();
        let key = ReceiptKey::new(&authorization.claims, &request, &digest).unwrap();
        assert!(claim(context, &principal, &key, None).await.unwrap());
        finish(
            context,
            &key,
            response(&request, &digest, status, None, None),
        )
        .await
        .unwrap();
        assert!(!claim(context, &principal, &key, None).await.unwrap());
    }

    let blocked_request = tests::file_request(b"retry");
    let blocked_digest = blocked_request.digest().unwrap();
    let blocked_key =
        ReceiptKey::new(&authorization.claims, &blocked_request, &blocked_digest).unwrap();
    assert!(
        claim(context, &principal, &blocked_key, None)
            .await
            .unwrap()
    );
    let blocked = response(
        &blocked_request,
        &blocked_digest,
        OfflineReplayStatus::Blocked,
        None,
        Some("The provider denied the operation without applying it"),
    );
    finish(context, &blocked_key, blocked.clone())
        .await
        .unwrap();
    let mut changed = blocked_key.clone();
    changed.digest = "changed-after-block".into();
    assert_eq!(
        claim(context, &principal, &changed, None)
            .await
            .unwrap_err()
            .status()
            .as_u16(),
        409,
        "a blocked retry preserves the immutable payload"
    );

    context.db.execute_raw(sql(
        r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes",initialized) VALUES ($1,10,true)"#,
        [authorization.claims.sub.clone().into()],
    )).await.unwrap();
    let reservation = PreparedUploadReservation::for_test(
        &authorization.claims.sub,
        &authorization.claims.project_id,
        (
            "content".into(),
            format!("offline-claim/{}", blocked_key.operation_id),
            5,
        ),
        10,
    );
    assert!(
        crate::quota::enforcing(),
        "run the lifecycle suite with quota enforcement"
    );
    assert_eq!(
        claim(context, &principal, &blocked_key, Some(reservation.clone()))
            .await
            .unwrap_err()
            .status()
            .as_u16(),
        402
    );
    assert_eq!(
        receipt(context.db, &blocked_key).await.unwrap(),
        Some(Some(blocked))
    );
    let grants = context
        .db
        .query_one_raw(sql(
            r#"SELECT COUNT(*) AS count FROM "StorageUploadGrant""#,
            Vec::<sea_orm::Value>::new(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        grants.try_get::<i64>("", "count").unwrap(),
        0,
        "quota failure rolls back reservations"
    );
    context
        .db
        .execute_raw(sql(
            r#"UPDATE "AccountCapacity" SET "storageBytes"=0 WHERE "payerId"=$1"#,
            [authorization.claims.sub.clone().into()],
        ))
        .await
        .unwrap();
    let (first, second) = flow_like_types::tokio::join!(
        claim(context, &principal, &blocked_key, Some(reservation.clone())),
        claim(context, &principal, &blocked_key, Some(reservation.clone()))
    );
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "quota recovery admits exactly one retry"
    );
    assert_eq!(receipt(context.db, &blocked_key).await.unwrap(), Some(None));
    assert!(
        !claim(context, &principal, &blocked_key, Some(reservation))
            .await
            .unwrap(),
        "an ambiguous retry is never dispatched again"
    );
    let usage = context
        .db
        .query_one_raw(sql(
            r#"SELECT "reservedStorageBytes" FROM "AccountCapacity" WHERE "payerId"=$1"#,
            [authorization.claims.sub.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        usage.try_get::<i64>("", "reservedStorageBytes").unwrap(),
        5,
        "concurrent claims reserve the object once"
    );
}

#[cfg(test)]
pub(super) async fn assert_revoked_attempt(
    context: &DeviceContext<'_>,
    authorization: &project::AuthorizedProject,
) {
    let principal = ReplayPrincipal::Instance(authorization.clone());
    let request = tests::file_request(b"later");
    let key = ReceiptKey::new(&authorization.claims, &request, &request.digest().unwrap()).unwrap();
    assert_eq!(
        claim(context, &principal, &key, None)
            .await
            .unwrap_err()
            .status()
            .as_u16(),
        401
    );
    assert!(receipt(context.db, &key).await.unwrap().is_none());
}

#[cfg(all(test, feature = "aws"))]
pub(super) async fn assert_file_http_lifecycle(
    context: &DeviceContext<'_>,
    session: &InstanceTokenResponse,
    workload: &SigningKey,
) {
    tests::file_http_lifecycle(context, session, workload).await;
}

#[cfg(test)]
#[path = "offline_desktop_tests.rs"]
mod desktop_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[cfg(feature = "aws")]
    #[derive(Clone)]
    struct HttpFixture {
        db: sea_orm::DatabaseConnection,
        dialect: crate::db::DbDialect,
        config: flow_like::hub::StandaloneConfig,
        domain: String,
        secure: bool,
        store: Arc<object_store::memory::InMemory>,
    }

    #[cfg(feature = "aws")]
    async fn file_http_handler(
        axum::extract::State(fixture): axum::extract::State<HttpFixture>,
        headers: HeaderMap,
        axum::Json(request): axum::Json<OfflineReplayRequest>,
    ) -> Result<axum::Json<OfflineReplayResponse>, ApiError> {
        let context = DeviceContext {
            db: &fixture.db,
            dialect: fixture.dialect,
            config: &fixture.config,
            domain: &fixture.domain,
            secure: fixture.secure,
        };
        let authorization = authorize(&context, &headers).await?;
        request.validate().map_err(invalid)?;
        let digest = request.digest().map_err(invalid)?;
        let key = ReceiptKey::new(&authorization.claims, &request, &digest)?;
        let OfflineResource::File { purpose, path } = &request.resource else {
            return Err(ApiError::FORBIDDEN);
        };
        let path = file_path(
            &authorization.claims.sub,
            &authorization.claims.project_id,
            *purpose,
            path,
        )?;
        let credentials = RuntimeCredentials::Aws(
            crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
                "meta",
                "content",
                "logs",
                "us-east-1",
            ),
        );
        let store = FlowLikeStore::Memory(fixture.store);
        Ok(axum::Json(
            run_file_attempt(
                &context,
                &ReplayPrincipal::Instance(authorization.clone()),
                &key,
                &path,
                &store,
                &credentials,
                &request,
                &digest,
                None,
            )
            .await?,
        ))
    }

    #[cfg(feature = "aws")]
    async fn send_file_http(
        router: &axum::Router,
        headers: HeaderMap,
        request: &OfflineReplayRequest,
    ) -> axum::response::Response {
        use tower::ServiceExt;
        let mut http = axum::http::Request::builder()
            .method("POST")
            .uri(OFFLINE_REPLAY_PATH)
            .body(axum::body::Body::from(serde_json::to_vec(request).unwrap()))
            .unwrap();
        *http.headers_mut() = headers;
        http.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        router.clone().oneshot(http).await.unwrap()
    }

    #[cfg(feature = "aws")]
    pub(super) async fn file_http_lifecycle(
        context: &DeviceContext<'_>,
        session: &InstanceTokenResponse,
        workload: &SigningKey,
    ) {
        let fixture = HttpFixture {
            db: context.db.clone(),
            dialect: context.dialect,
            config: context.config.clone(),
            domain: context.domain.into(),
            secure: context.secure,
            store: Arc::new(object_store::memory::InMemory::new()),
        };
        let router = axum::Router::new()
            .route(OFFLINE_REPLAY_PATH, axum::routing::post(file_http_handler))
            .layer(axum::extract::DefaultBodyLimit::max(
                MAX_OFFLINE_REPLAY_HTTP_BYTES,
            ))
            .with_state(fixture.clone());
        let headers = || {
            let proof = sign_dpop(
                &DpopProof {
                    jti: uuid::Uuid::new_v4().to_string(),
                    htm: "POST".into(),
                    htu: endpoint_url(
                        context.config.api_base_url.as_deref().unwrap(),
                        OFFLINE_REPLAY_PATH,
                    )
                    .unwrap(),
                    iat: now(),
                    ath: Some(access_token_hash(&session.access_token)),
                    nonce: Some(session.dpop_nonce.clone()),
                },
                workload,
            )
            .unwrap();
            HeaderMap::from_iter([
                (
                    "authorization".parse::<axum::http::HeaderName>().unwrap(),
                    format!("DPoP {}", session.access_token).parse().unwrap(),
                ),
                ("dpop".parse().unwrap(), proof.parse().unwrap()),
            ])
        };
        let request = file_request(b"device");
        assert_eq!(
            send_file_http(&router, HeaderMap::new(), &request)
                .await
                .status()
                .as_u16(),
            401
        );
        let first_headers = headers();
        let result = send_file_http(&router, first_headers.clone(), &request).await;
        assert_eq!(result.status().as_u16(), 200);
        let first: OfflineReplayResponse = serde_json::from_slice(
            &axum::body::to_bytes(result.into_body(), MAX_OFFLINE_REPLAY_HTTP_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first.status, OfflineReplayStatus::Applied);
        assert_eq!(
            send_file_http(&router, first_headers, &request)
                .await
                .status()
                .as_u16(),
            401,
            "DPoP proofs cannot be replayed"
        );
        let authorization = authorize(context, &headers()).await.unwrap();
        let path = file_path(
            &authorization.claims.sub,
            &authorization.claims.project_id,
            StoragePurpose::Files,
            "file.txt",
        )
        .unwrap();
        fixture
            .store
            .put(&path, b"later-writer".to_vec().into())
            .await
            .unwrap();
        let result = send_file_http(&router, headers(), &request).await;
        assert_eq!(result.status().as_u16(), 200);
        let second: OfflineReplayResponse = serde_json::from_slice(
            &axum::body::to_bytes(result.into_body(), MAX_OFFLINE_REPLAY_HTTP_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            first, second,
            "fresh DPoP retrieves the durable mutation receipt"
        );
        assert_eq!(
            fixture
                .store
                .get(&path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            b"later-writer"
        );
        let mut different = file_request(b"different");
        different.operation_id = request.operation_id.clone();
        assert_eq!(
            send_file_http(&router, headers(), &different)
                .await
                .status()
                .as_u16(),
            409
        );

        // Simulate a process dying after the provider accepted the write, before
        // SQL received its result. A fresh request may inspect but never resend it.
        let ambiguous = file_request(b"ambiguous");
        let digest = ambiguous.digest().unwrap();
        let key = ReceiptKey::new(&authorization.claims, &ambiguous, &digest).unwrap();
        assert!(
            claim(
                context,
                &ReplayPrincipal::Instance(authorization.clone()),
                &key,
                None
            )
            .await
            .unwrap()
        );
        fixture
            .store
            .put(&path, b"ambiguous".to_vec().into())
            .await
            .unwrap();
        fixture.store.delete(&path).await.unwrap();
        let result = send_file_http(&router, headers(), &ambiguous).await;
        assert_eq!(result.status().as_u16(), 200);
        let retained: OfflineReplayResponse = serde_json::from_slice(
            &axum::body::to_bytes(result.into_body(), MAX_OFFLINE_REPLAY_HTTP_BYTES)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(retained.status, OfflineReplayStatus::OutcomeUnknown);
        assert!(
            matches!(
                fixture.store.head(&path).await,
                Err(object_store::Error::NotFound { .. })
            ),
            "ambiguous create must not resurrect deleted cloud content"
        );
    }

    pub(super) fn file_request(bytes: &[u8]) -> OfflineReplayRequest {
        OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: OfflineResource::File {
                purpose: StoragePurpose::Files,
                path: "file.txt".into(),
            },
            expected: OfflineExpected::FileAbsent,
            mutation: OfflineMutation::FilePut {
                data_base64: STANDARD.encode(bytes),
                sha256: format!("{:x}", Sha256::digest(bytes)),
            },
        }
    }

    #[flow_like_types::tokio::test]
    async fn conditional_create_preserves_concurrent_cloud_content() {
        let store = object_store::memory::InMemory::new();
        let path = Path::from("file.txt");
        let request = file_request(b"device");
        let digest = request.digest().unwrap();
        store
            .put(&path, b"concurrent".to_vec().into())
            .await
            .unwrap();
        assert_eq!(
            put_file(&store, &path, &request, &digest).await.status,
            OfflineReplayStatus::Conflict
        );
        assert_eq!(
            store
                .get(&path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            b"concurrent"
        );
    }

    #[flow_like_types::tokio::test]
    async fn conditional_overwrite_requires_the_queued_revision() {
        let store = object_store::memory::InMemory::new();
        let path = Path::from("file.txt");
        let original = store.put(&path, b"original".to_vec().into()).await.unwrap();
        let mut request = file_request(b"device");
        request.expected = OfflineExpected::FileRevision {
            e_tag: original.e_tag,
            version: original.version,
        };
        store
            .put(&path, b"concurrent".to_vec().into())
            .await
            .unwrap();
        let digest = request.digest().unwrap();
        assert_eq!(
            put_file(&store, &path, &request, &digest).await.status,
            OfflineReplayStatus::Conflict
        );
        assert_eq!(
            store
                .get(&path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            b"concurrent"
        );
    }

    #[test]
    fn exact_user_prefix_and_traversal_are_enforced() {
        assert_eq!(
            file_path("auth0|owner", "project", StoragePurpose::User, "report.txt")
                .unwrap()
                .as_ref(),
            "users/auth0%7Cowner/apps/project/report.txt"
        );
        for path in [
            "../other",
            "/users/other/apps/project/report",
            "a/../../b",
            "a\\b",
        ] {
            assert!(file_path("owner", "project", StoragePurpose::User, path).is_err());
        }
    }

    #[cfg(feature = "aws")]
    #[test]
    fn aws_etags_are_not_treated_as_incarnation_fences() {
        let c = RuntimeCredentials::Aws(
            crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
                "meta",
                "content",
                "logs",
                "us-east-1",
            ),
        );
        let expected = OfflineExpected::FileRevision {
            e_tag: Some("same-content".into()),
            version: Some("version-id".into()),
        };
        assert!(file_support(&c, &expected, &OfflineMutation::FileDelete).is_some());
        assert!(file_support(&c, &expected, &file_request(b"overwrite").mutation).is_some());
        assert!(
            file_support(
                &c,
                &OfflineExpected::FileAbsent,
                &file_request(b"new").mutation
            )
            .is_none()
        );
    }

    #[test]
    fn volatile_table_mutations_are_rejected_before_admission() {
        let mut request = file_request(b"");
        request.resource = OfflineResource::Table {
            purpose: StoragePurpose::Storage,
            database: "db".into(),
            table: "rows".into(),
        };
        request.expected = OfflineExpected::TableVersion {
            version: 1,
            fingerprint: Some(format!("blake3:{}", "0".repeat(64))),
        };
        request.mutation = OfflineMutation::TableUpdate {
            filter: "id = 1".into(),
            updates: std::collections::BTreeMap::from([("updated_at".into(), "now()".into())]),
        };
        assert!(table_mutation(&request).is_err());
        request.mutation = OfflineMutation::TableDelete {
            filter: "id = 1".into(),
        };
        assert!(table_mutation(&request).is_ok());
    }
}

fn table_mutation(
    request: &OfflineReplayRequest,
) -> Result<flow_like_storage::databases::vector::offline_replay::ReplayMutation, ApiError> {
    use flow_like_storage::databases::vector::offline_replay::{
        ReplayMutation, validate_expression,
    };
    let rows_valid = |rows: &Vec<serde_json::Value>| {
        !rows.is_empty() && rows.iter().all(serde_json::Value::is_object)
    };
    match &request.mutation {
        OfflineMutation::TableInsert { rows } if rows_valid(rows) => Ok(ReplayMutation::Insert {
            items: rows.clone(),
        }),
        OfflineMutation::TableUpsert { id_field, rows } if rows_valid(rows) => {
            validate_instance_identifier(id_field).map_err(invalid)?;
            Ok(ReplayMutation::Upsert {
                items: rows.clone(),
                id_field: id_field.clone(),
            })
        }
        OfflineMutation::TableUpdate { filter, updates } if !updates.is_empty() => {
            validate_expression(filter).map_err(|_| {
                ApiError::bad_request("Offline filters must use deterministic scalar expressions")
            })?;
            for (column, expression) in updates {
                validate_instance_identifier(column).map_err(invalid)?;
                validate_expression(expression).map_err(|_| {
                    ApiError::bad_request(
                        "Offline updates must use deterministic scalar expressions",
                    )
                })?;
            }
            Ok(ReplayMutation::Update {
                filter: filter.clone(),
                updates: updates
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            })
        }
        OfflineMutation::TableDelete { filter } => {
            validate_expression(filter).map_err(|_| {
                ApiError::bad_request("Offline filters must use deterministic scalar expressions")
            })?;
            Ok(ReplayMutation::Delete {
                filter: filter.clone(),
            })
        }
        _ => Err(ApiError::bad_request(
            "Offline table rows or mutation are invalid",
        )),
    }
}

fn table_outcome(
    request: &OfflineReplayRequest,
    digest: &str,
    outcome: flow_like_storage::databases::vector::offline_replay::ReplayOutcome,
) -> OfflineReplayResponse {
    use flow_like_storage::databases::vector::offline_replay::ReplayOutcome;
    match outcome {
        ReplayOutcome::Applied {
            version,
            fingerprint,
        } => response(
            request,
            digest,
            OfflineReplayStatus::Applied,
            Some(OfflineExpected::TableVersion {
                version,
                fingerprint: Some(fingerprint),
            }),
            None,
        ),
        ReplayOutcome::Conflict { actual_version } => response(
            request,
            digest,
            OfflineReplayStatus::Conflict,
            Some(OfflineExpected::TableVersion {
                version: actual_version,
                fingerprint: None,
            }),
            Some("The cloud table differs from the queued base revision"),
        ),
        ReplayOutcome::NotApplied | ReplayOutcome::Unknown { .. } => unknown(request, digest),
    }
}

#[allow(clippy::too_many_arguments)]
async fn replay_table(
    state: &AppState,
    context: &DeviceContext<'_>,
    principal: &ReplayPrincipal<'_>,
    key: &ReceiptKey,
    master: &RuntimeCredentials,
    mutation: ReplayMutation,
    request: OfflineReplayRequest,
    digest: String,
    previous: bool,
) -> Result<OfflineReplayResponse, ApiError> {
    use flow_like_storage::databases::vector::offline_replay::{
        self as lance_replay, ReplayMarker,
    };
    let OfflineResource::Table { purpose, table, .. } = &request.resource else {
        unreachable!()
    };
    let OfflineExpected::TableVersion {
        version,
        fingerprint,
    } = &request.expected
    else {
        unreachable!()
    };
    let marker = ReplayMarker {
        operation_id: request.operation_id.clone(),
        digest: flow_like_storage::blake3::hash(&serde_json::to_vec(&(
            &key.grant_id,
            key.authz_version,
            &digest,
        ))?)
        .to_hex()
        .to_string(),
        expected_version: *version,
        expected_fingerprint: fingerprint.clone(),
    };
    if (*version == 0 && fingerprint.is_some()) || (*version > 0 && fingerprint.is_none()) {
        return Ok(response(
            &request,
            &digest,
            OfflineReplayStatus::Unsupported,
            None,
            Some("Existing tables require their exact manifest fingerprint"),
        ));
    }
    if *version == 0 && matches!(principal, ReplayPrincipal::Desktop { .. }) {
        return Ok(response(
            &request,
            &digest,
            OfflineReplayStatus::Unsupported,
            None,
            Some("Desktop offline changes can only modify tables that exist in the cloud"),
        ));
    }
    let shared = master.into_shared_credentials();
    let builder = match purpose {
        StoragePurpose::Storage => shared.to_db(principal.project_id()).await,
        StoragePurpose::User => {
            shared
                .to_db_scoped(principal.sub(), principal.project_id())
                .await
        }
        _ => return Err(ApiError::FORBIDDEN),
    }
    .map_err(|_| ApiError::service_unavailable("Cannot authorize the offline replay database"))?;
    let connection = builder
        .execute()
        .await
        .map_err(|_| ApiError::service_unavailable("The offline replay database is unavailable"))?;
    let existing = match connection.open_table(table).execute().await {
        Ok(table) => Some(table),
        Err(flow_like_storage::lancedb::Error::TableNotFound { .. }) => None,
        Err(_) => {
            return Ok(if previous {
                unknown(&request, &digest)
            } else {
                response(
                    &request,
                    &digest,
                    OfflineReplayStatus::Blocked,
                    None,
                    Some("The cloud table is unavailable"),
                )
            });
        }
    };
    if previous {
        let Some(existing) = existing else {
            return Ok(unknown(&request, &digest));
        };
        let value = match lance_replay::reconcile(&existing, &marker).await {
            Ok(outcome) => table_outcome(&request, &digest, outcome),
            Err(_) => unknown(&request, &digest),
        };
        return finish_dispatched(context, principal, key, &request, value).await;
    }
    if existing.is_none() && *version != 0 {
        return Ok(response(
            &request,
            &digest,
            OfflineReplayStatus::Conflict,
            Some(OfflineExpected::TableVersion {
                version: 0,
                fingerprint: None,
            }),
            Some("The queued table no longer exists"),
        ));
    }
    if *version == 0
        && !matches!(
            request.mutation,
            OfflineMutation::TableInsert { .. } | OfflineMutation::TableUpsert { .. }
        )
    {
        return Ok(response(
            &request,
            &digest,
            OfflineReplayStatus::Unsupported,
            None,
            Some("Creating an offline table requires schema-bearing insert or upsert rows"),
        ));
    }
    // Lance's physical bytes include fragments, indexes and manifests. The
    // logical payload is a quota admission floor, not an exact byte reservation.
    if !matches!(request.mutation, OfflineMutation::TableDelete { .. }) {
        let bytes = i64::try_from(serde_json::to_vec(&request.mutation)?.len()).map_err(invalid)?;
        if let Err(error) = crate::capacity::check_storage_write(
            state,
            principal.project_id(),
            principal.sub(),
            bytes,
        )
        .await
        {
            return Ok(response(
                &request,
                &digest,
                OfflineReplayStatus::Blocked,
                None,
                Some(
                    error
                        .public_message()
                        .unwrap_or("Storage quota admission is unavailable"),
                ),
            ));
        }
    }
    if !claim(context, principal, key, None).await? {
        return Ok(receipt(context.db, key)
            .await?
            .flatten()
            .unwrap_or_else(|| unknown(&request, &digest)));
    }
    if let Err(error) = principal.recheck(context, &request.resource).await {
        return finish(
            context,
            key,
            response(
                &request,
                &digest,
                OfflineReplayStatus::Blocked,
                None,
                Some(
                    error
                        .public_message()
                        .unwrap_or("Project authorization changed before replay"),
                ),
            ),
        )
        .await;
    }
    let outcome = if *version == 0 {
        let rows = match &request.mutation {
            OfflineMutation::TableInsert { rows } | OfflineMutation::TableUpsert { rows, .. } => {
                rows.clone()
            }
            _ => unreachable!(),
        };
        lance_replay::create(&connection, table, &marker, rows).await
    } else {
        lance_replay::replay(
            existing.as_ref().expect("existing table was checked"),
            &marker,
            mutation,
        )
        .await
    };
    let value = match outcome {
        Ok(outcome) => table_outcome(&request, &digest, outcome),
        Err(_) => unknown(&request, &digest),
    };
    finish_dispatched(context, principal, key, &request, value).await
}
