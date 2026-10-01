use super::{context, human_owner, management, repository};
use crate::{
    cache::{PlatformCache, Reservation},
    db::{RetryPolicy, retry_transaction},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement, Value};
use serde::{Deserialize, Serialize};
use std::result::Result;

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}
fn invalid(e: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(e.to_string())
}
fn key(id: &str) -> Result<Ed25519PublicKey, ApiError> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(id)
        .map_err(invalid)?
        .try_into()
        .map_err(|_| invalid("Invalid fleet controller"))?;
    if URL_SAFE_NO_PAD.encode(bytes) != id {
        return Err(invalid("Invalid fleet controller"));
    }
    Ed25519PublicKey::from_bytes(bytes).map_err(invalid)
}
/// Every replacement changes the reader digest the device seals to, so it
/// invalidates that reader's in-flight publication.
const MIN_READER_RENEWAL_SECONDS: i64 = 60 * 60;
const READER_CHANGE_NAMESPACE: &str = "device-fleet-reader-change";
/// Stale non-deleted rows can outnumber live readers; they are filtered after loading.
const MAX_READER_ROWS: u64 = 1024;

fn issued_at(compact: &str) -> Option<i64> {
    let payload = URL_SAFE_NO_PAD.decode(compact.split('.').nth(1)?).ok()?;
    serde_json::from_slice::<FleetReader>(&payload)
        .ok()
        .map(|reader| reader.issued_at)
}

/// `accepted_at` is when the server last accepted a change to this reader. The
/// grantee chooses the signed issued_at, so it only raises that floor: a
/// declaration signed long ago, such as a retried pending one, is still accepted
/// once the interval has passed, but backdating cannot skip it.
fn check_renewal_interval(
    previous_jws: &str,
    accepted_at: Option<i64>,
    now: i64,
) -> Result<(), ApiError> {
    let Some(previous) = issued_at(previous_jws).max(accepted_at) else {
        return Ok(());
    };
    let allowed_at = previous.saturating_add(MIN_READER_RENEWAL_SECONDS);
    if now < allowed_at {
        return Err(ApiError::too_many_requests(format!(
            "Fleet reader renewals must be at least {MIN_READER_RENEWAL_SECONDS} seconds apart; the current reader changed at {previous}, so a replacement is accepted from {allowed_at}"
        )));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct ReaderChange {
    accepted_at: i64,
}

/// A cross-replica record of a reader's last accepted change, kept for one
/// renewal interval. Without the platform cache only the signed floor applies.
enum ReaderChangeClaim {
    Owned(PlatformCache),
    Held(i64),
    Unavailable,
}

impl ReaderChangeClaim {
    async fn acquire(state: &AppState, key: &str, now: i64) -> Self {
        let cache = match state.cache.platform().await {
            Ok(cache) => cache,
            Err(error) => {
                tracing::warn!(%error, "Platform cache unavailable; fleet reader renewals fall back to the signed issue time");
                return Self::Unavailable;
            }
        };
        let ttl = std::time::Duration::from_secs(MIN_READER_RENEWAL_SECONDS as u64);
        match cache
            .try_insert(
                READER_CHANGE_NAMESPACE,
                key,
                &ReaderChange { accepted_at: now },
                ttl,
            )
            .await
        {
            Ok(Reservation::Acquired) => Self::Owned(cache),
            Ok(Reservation::Held(change)) => Self::Held(change.accepted_at),
            Err(error) => {
                tracing::warn!(%error, "Fleet reader change record failed; renewals fall back to the signed issue time");
                Self::Unavailable
            }
        }
    }

    fn accepted_at(&self) -> Option<i64> {
        match self {
            Self::Held(accepted_at) => Some(*accepted_at),
            Self::Owned(_) | Self::Unavailable => None,
        }
    }

    /// An owned record stays only when this request changed the reader.
    async fn settle(self, key: &str, changed: bool) {
        let Self::Owned(cache) = self else {
            return;
        };
        if changed {
            return;
        }
        if let Err(error) = cache.delete(READER_CHANGE_NAMESPACE, key).await {
            tracing::warn!(%error, "Could not release an unused fleet reader change record; renewals of this reader wait for it to expire");
        }
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/{id}/fleet/readers/{key}",
            get(reader_get).put(reader_put).delete(reader_delete),
        )
        .route("/{id}/fleet/recipients", post(recipients))
        .route(
            "/{id}/fleet/snapshots",
            post(upload).layer(DefaultBodyLimit::max(400 * 1024)),
        )
        .route("/{id}/fleet/snapshots/{key}", get(download))
}
async fn policy(
    tx: &DatabaseTransaction,
    manifest: &OnboardingManifest,
    now: i64,
) -> Result<Option<(String, ManagementPolicy)>, ApiError> {
    let row = tx.query_one_raw(sql(r#"SELECT "policyJws" FROM "DeviceManagementPolicy" WHERE "deviceId"=$1 ORDER BY version DESC LIMIT 1"#, [manifest.device_id.clone().into()])).await?;
    let compact = row
        .map(|r| r.try_get::<String>("", "policyJws"))
        .transpose()?;
    Ok(compact.and_then(|jws| {
        verify_management_policy(&jws, &manifest.owner_invitation_key, now)
            .ok()
            .map(|p| (jws, p))
    }))
}
async fn load_reader(
    tx: &DatabaseTransaction,
    id: &str,
    user: &str,
    key_id: &str,
) -> Result<Option<FleetReaderState>, ApiError> {
    tx.query_one_raw(sql(r#"SELECT "readerJws",revision,deleted FROM "DeviceFleetReader" WHERE "deviceId"=$1 AND "userId"=$2 AND "keyId"=$3"#, [id.into(),user.into(),key_id.into()])).await?.map(|r| Ok(FleetReaderState { reader_jws:r.try_get("","readerJws")?, revision:r.try_get::<i64>("","revision")? as u64, deleted:r.try_get("","deleted")? })).transpose()
}
async fn live_readers(
    tx: &DatabaseTransaction,
    id: &str,
    manifest: &OnboardingManifest,
    policy: Option<&(String, ManagementPolicy)>,
    now: i64,
) -> Result<Vec<(String, String, FleetReaderState)>, ApiError> {
    let rows = tx.query_all_raw(sql(&format!(r#"SELECT r."userId",r."keyId",r."readerJws",r.revision FROM "DeviceFleetReader" r JOIN "User" u ON u.id=r."userId" WHERE r."deviceId"=$1 AND r.deleted=false AND u.status='ACTIVE' ORDER BY r."userId",r."keyId" LIMIT {MAX_READER_ROWS}"#), [id.into()])).await?;
    let mut live = vec![];
    for row in rows {
        let (user, key_id) = (
            row.try_get::<String>("", "userId")?,
            row.try_get::<String>("", "keyId")?,
        );
        let state = FleetReaderState {
            reader_jws: row.try_get("", "readerJws")?,
            revision: row.try_get::<i64>("", "revision")? as u64,
            deleted: false,
        };
        if authorize_reader(&state, &key(&key_id)?, &user, manifest, policy, now).is_ok() {
            live.push((user, key_id, state));
        }
    }
    Ok(live)
}
fn authorize_reader(
    state: &FleetReaderState,
    key: &Ed25519PublicKey,
    user: &str,
    manifest: &OnboardingManifest,
    current: Option<&(String, ManagementPolicy)>,
    now: i64,
) -> Result<(FleetReader, Vec<FleetAudience>), ApiError> {
    if state.deleted {
        return Err(ApiError::FORBIDDEN);
    }
    let reader =
        verify_fleet_reader(&state.reader_jws, key, now).map_err(|_| ApiError::FORBIDDEN)?;
    if reader.user_id != user || reader.revision != state.revision {
        return Err(ApiError::FORBIDDEN);
    }
    let audiences = fleet_audiences(
        &reader,
        &state.reader_jws,
        manifest,
        current.map(|(j, p)| (j.as_str(), p)),
        now,
    );
    if audiences.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }
    Ok((reader, audiences))
}
#[derive(Clone)]
enum ReaderAction {
    Get,
    Put(String),
    Delete(u64),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaderPut {
    reader_jws: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaderDelete {
    revision: u64,
}
async fn reader_get(
    State(s): State<AppState>,
    Extension(u): Extension<AppUser>,
    Path((id, k)): Path<(String, String)>,
) -> Result<Json<FleetReaderState>, ApiError> {
    reader_action(s, u, id, k, ReaderAction::Get)
        .await
        .map(Json)
}
async fn reader_put(
    State(s): State<AppState>,
    Extension(u): Extension<AppUser>,
    Path((id, k)): Path<(String, String)>,
    Json(v): Json<ReaderPut>,
) -> Result<Json<FleetReaderState>, ApiError> {
    reader_action(s, u, id, k, ReaderAction::Put(v.reader_jws))
        .await
        .map(Json)
}
async fn reader_delete(
    State(s): State<AppState>,
    Extension(u): Extension<AppUser>,
    Path((id, k)): Path<(String, String)>,
    Json(v): Json<ReaderDelete>,
) -> Result<Json<FleetReaderState>, ApiError> {
    reader_action(s, u, id, k, ReaderAction::Delete(v.revision))
        .await
        .map(Json)
}
async fn reader_action(
    state: AppState,
    user: AppUser,
    id: String,
    key_id: String,
    action: ReaderAction,
) -> Result<FleetReaderState, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    let manifest = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    let change_key = format!("{id}:{user}:{key_id}");
    let claim = match &action {
        ReaderAction::Put(_) => Some(
            ReaderChangeClaim::acquire(&state, &change_key, chrono::Utc::now().timestamp()).await,
        ),
        ReaderAction::Get | ReaderAction::Delete(_) => None,
    };
    let accepted_at = claim.as_ref().and_then(ReaderChangeClaim::accepted_at);
    let result = retain_reader(
        &state.db,
        state.db_dialect,
        device,
        manifest,
        user,
        key_id,
        accepted_at,
        action,
    )
    .await;
    if let Some(claim) = claim {
        claim
            .settle(&change_key, matches!(result, Ok((_, true))))
            .await;
    }
    result.map(|(reader, _)| reader)
}
/// Returns the reader and whether this call changed it.
#[allow(clippy::too_many_arguments)]
async fn retain_reader(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    device: repository::Device,
    manifest: OnboardingManifest,
    user: String,
    key_id: String,
    accepted_at: Option<i64>,
    action: ReaderAction,
) -> Result<(FleetReaderState, bool), ApiError> {
    let id = device.status.device_id.clone();
    let key = key(&key_id)?;
    if let ReaderAction::Put(jws) = &action {
        let reader =
            verify_fleet_reader(jws, &key, chrono::Utc::now().timestamp()).map_err(invalid)?;
        if reader.user_id != user
            || reader.api_base_url != manifest.api_base_url
            || reader.device_id != id
        {
            return Err(ApiError::FORBIDDEN);
        }
    }
    retry_transaction(db,dialect,None,&RetryPolicy::default(),move|tx| {
        let (id,user,key_id,key,action,manifest,device)=(id.clone(),user.clone(),key_id.clone(),key.clone(),action.clone(),manifest.clone(),device.clone());
        Box::pin(async move {
            repository::lock_active_device(tx,&id,device.status.auth_epoch).await?;
            repository::active_account(tx,&user).await?;
            let now=chrono::Utc::now().timestamp();
            let policy=policy(tx,&manifest,now).await?;
            let old=load_reader(tx,&id,&user,&key_id).await?;
            let is_get=matches!(action,ReaderAction::Get);
            let output=match action {
                ReaderAction::Get => old.ok_or(ApiError::NOT_FOUND)?,
                ReaderAction::Delete(revision) => {
                    let mut old=old.ok_or(ApiError::NOT_FOUND)?;
                    if old.deleted && revision==old.revision {return Ok((old,false));}
                    if revision!=old.revision+1 {return Err(ApiError::conflict("Fleet reader changed; reload before removing it"));}
                    old.revision=revision; old.deleted=true;
                    old
                },
                ReaderAction::Put(jws) => {
                    let reader=verify_fleet_reader(&jws,&key,now).map_err(invalid)?;
                    let next=FleetReaderState {reader_jws:jws,revision:reader.revision,deleted:false};
                    authorize_reader(&next,&key,&user,&manifest,policy.as_ref(),now)?;
                    if let Some(old)=&old {
                        if old.revision==next.revision && old.reader_jws==next.reader_jws && !old.deleted {return Ok((next,false));}
                        if next.revision!=old.revision+1 {return Err(ApiError::conflict("Fleet reader changed; reload before replacing it"));}
                        check_renewal_interval(&old.reader_jws,accepted_at,now)?;
                    } else {
                        if next.revision!=1 {return Err(ApiError::conflict("Fleet reader genesis revision required"));}
                        // Deleted rows stay as replay floors; only live readers use capacity.
                        if live_readers(tx,&id,&manifest,policy.as_ref(),now).await?.len()>=MAX_FLEET_READERS {return Err(invalid("This device reached its fleet reader key limit"));}
                    }
                    next
                }
            };
            if is_get {return Ok((output,false));}
            tx.execute_raw(sql(r#"INSERT INTO "DeviceFleetReader"("deviceId","userId","keyId",revision,"readerJws",deleted) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT("deviceId","userId","keyId") DO UPDATE SET revision=excluded.revision,"readerJws"=excluded."readerJws",deleted=excluded.deleted"#,[id.clone().into(),user.clone().into(),key_id.clone().into(),(output.revision as i64).into(),output.reader_jws.clone().into(),output.deleted.into()])).await?;
            // Key renewal or deletion removes the old ciphertext, with its replay floor retained by the device head.
            tx.execute_raw(sql(r#"DELETE FROM "DeviceFleetSnapshot" WHERE "deviceId"=$1 AND "userId"=$2 AND "keyId"=$3"#,[id.into(),user.into(),key_id.into()])).await?;
            Ok((output,true))
        })
    }).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    client_assertion: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    client_assertion: String,
    bundle: EncryptedFleetSnapshot,
}
async fn head(tx: &DatabaseTransaction, id: &str) -> Result<FleetHead, ApiError> {
    Ok(tx
        .query_one_raw(sql(
            r#"SELECT sequence,digest FROM "DeviceFleetHead" WHERE "deviceId"=$1"#,
            [id.into()],
        ))
        .await?
        .map(|r| {
            Ok::<_, ApiError>(FleetHead {
                sequence: r.try_get::<i64>("", "sequence")? as u64,
                manifest_digest: Some(r.try_get("", "digest")?),
            })
        })
        .transpose()?
        .unwrap_or_default())
}
async fn recipients(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<Admission>,
) -> Result<Json<FleetRecipients>, ApiError> {
    let device = super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/fleet/recipients"),
    )
    .await?;
    let manifest = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    retry_transaction(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let (id, device, manifest) = (id.clone(), device.clone(), manifest.clone());
            Box::pin(async move {
                repository::lock_active_device(tx, &id, device.status.auth_epoch).await?;
                let now = chrono::Utc::now().timestamp();
                let policy = policy(tx, &manifest, now).await?;
                let readers = live_readers(tx, &id, &manifest, policy.as_ref(), now)
                    .await?
                    .into_iter()
                    .take(MAX_FLEET_READERS)
                    .map(|(_, _, state)| state.reader_jws)
                    .collect();
                Ok(Json(FleetRecipients {
                    readers,
                    policy_jws: policy.map(|(j, _)| j),
                    head: head(tx, &id).await?,
                }))
            })
        },
    )
    .await
}
async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<Upload>,
) -> Result<Json<FleetHead>, ApiError> {
    let device = super::registered_assertion(
        &context(&state),
        &id,
        &request.client_assertion,
        &format!("/devices/{id}/fleet/snapshots"),
    )
    .await?;
    let onboarding = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    retain_snapshot(
        &state.db,
        state.db_dialect,
        device,
        onboarding,
        request.bundle,
    )
    .await
    .map(Json)
}
async fn retain_snapshot(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    device: repository::Device,
    onboarding: OnboardingManifest,
    bundle: EncryptedFleetSnapshot,
) -> Result<FleetHead, ApiError> {
    let id = device.status.device_id.clone();
    let manifest =
        verify_fleet_snapshot(&bundle, &device.status.identity.telemetry_key).map_err(invalid)?;
    if manifest.device_id != id
        || manifest.api_base_url != onboarding.api_base_url
        || manifest.observed_at > chrono::Utc::now().timestamp() + 30
    {
        return Err(invalid("Invalid fleet publisher binding"));
    }
    let key_id = URL_SAFE_NO_PAD.encode(manifest.controller_key.to_bytes().map_err(invalid)?);
    let digest = compact_digest(&bundle.manifest_jws);
    let stream = fleet_digest(&serde_json::to_vec(&(
        &manifest.user_id,
        &key_id,
        &manifest.audience.grant_id,
        &manifest.audience.scope,
        manifest.audience.kind,
    ))?);
    let encoded = serde_json::to_string(&bundle)?;
    let manifest_jws = bundle.manifest_jws;
    retry_transaction(db,dialect,None,&RetryPolicy::default(),move|tx|{
        let manifest_jws=manifest_jws.clone();
        let (id,device,onboarding,manifest,key_id,digest,stream,encoded)=(id.clone(),device.clone(),onboarding.clone(),manifest.clone(),key_id.clone(),digest.clone(),stream.clone(),encoded.clone());
        Box::pin(async move {
            repository::lock_active_device(tx,&id,device.status.auth_epoch).await?;
            repository::active_account(tx,&manifest.user_id).await?;
            let now=chrono::Utc::now().timestamp();let policy=policy(tx,&onboarding,now).await?;
            let reader=load_reader(tx,&id,&manifest.user_id,&key_id).await?.ok_or(ApiError::FORBIDDEN)?;
            let (_,audiences)=authorize_reader(&reader,&manifest.controller_key,&manifest.user_id,&onboarding,policy.as_ref(),now)?;
            if !audiences.contains(&manifest.audience) {return Err(ApiError::FORBIDDEN);}
            let previous=head(tx,&id).await?;
            if previous.sequence==manifest.sequence && previous.manifest_digest.as_ref()==Some(&digest) {return Ok(previous);}
            if previous.sequence+1!=manifest.sequence || previous.manifest_digest!=manifest.previous_digest {return Err(ApiError::conflict("Fleet sequence does not extend the device publication chain"));}
            let readers:std::collections::BTreeMap<_,_>=live_readers(tx,&id,&onboarding,policy.as_ref(),now).await?.into_iter().map(|(user,key_id,state)|((user,key_id),state)).collect();
            let rows=tx.query_all_raw(sql(r#"SELECT "streamId","manifestJws","updatedAt" FROM "DeviceFleetSnapshot" WHERE "deviceId"=$1 LIMIT 128"#,[id.clone().into()])).await?;
            let mut retained=0; let mut exists=false;
            for row in rows {
                let old_stream=row.try_get::<String>("","streamId")?;
                let old=verify_fleet_manifest(&row.try_get::<String>("","manifestJws")?,&device.status.identity.telemetry_key).map_err(invalid)?;
                let old_key=URL_SAFE_NO_PAD.encode(old.controller_key.to_bytes().map_err(invalid)?);
                let valid=if let Some(reader)=readers.get(&(old.user_id.clone(),old_key)) { authorize_reader(reader,&old.controller_key,&old.user_id,&onboarding,policy.as_ref(),now).is_ok_and(|(_,a)|a.contains(&old.audience)) }else{false};
                if !valid {tx.execute_raw(sql(r#"DELETE FROM "DeviceFleetSnapshot" WHERE "deviceId"=$1 AND "streamId"=$2"#,[id.clone().into(),old_stream.into()])).await?;continue;}
                retained+=1;
                if old_stream==stream {exists=true;let minimum=if manifest.audience.kind==FleetKind::Metrics {25}else{5}; if row.try_get::<i64>("","updatedAt")?+minimum>now {return Err(ApiError::conflict("Fleet publication is too frequent; retry after the publication interval"));}}
            }
            if !exists && retained>=MAX_FLEET_STREAMS {return Err(invalid("This device reached its retained fleet scope limit"));}
            tx.execute_raw(sql(r#"INSERT INTO "DeviceFleetSnapshot"("deviceId","streamId","userId","keyId",bundle,"updatedAt","manifestJws") VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT("deviceId","streamId") DO UPDATE SET bundle=excluded.bundle,"updatedAt"=excluded."updatedAt","manifestJws"=excluded."manifestJws""#,[id.clone().into(),stream.into(),manifest.user_id.into(),key_id.into(),encoded.into(),now.into(),manifest_jws.into()])).await?;
            tx.execute_raw(sql(r#"INSERT INTO "DeviceFleetHead"("deviceId",sequence,digest) VALUES($1,$2,$3) ON CONFLICT("deviceId") DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest"#,[id.into(),(manifest.sequence as i64).into(),digest.clone().into()])).await?;
            Ok(FleetHead {sequence:manifest.sequence,manifest_digest:Some(digest)})
        })
    }).await
}
async fn download(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((id, key_id)): Path<(String, String)>,
) -> Result<Json<FleetView>, ApiError> {
    let user = human_owner(&state, &user).await?;
    let (device, _) = management::admitted_device(&state, &user, &id).await?;
    let manifest = super::repository(&context(&state))
        .enrollment(&device.receipt.enrollment_id)
        .await?
        .manifest;
    retained_view(&state.db, state.db_dialect, device, manifest, user, key_id)
        .await
        .map(Json)
}
async fn retained_view(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    device: repository::Device,
    manifest: OnboardingManifest,
    user: String,
    key_id: String,
) -> Result<FleetView, ApiError> {
    let id = device.status.device_id.clone();
    let key = key(&key_id)?;
    retry_transaction(db,dialect,None,&RetryPolicy::default(),move|tx|{
        let (id,key_id,user,device,manifest,key)=(id.clone(),key_id.clone(),user.clone(),device.clone(),manifest.clone(),key.clone());
        Box::pin(async move {
            repository::lock_active_device(tx,&id,device.status.auth_epoch).await?;repository::active_account(tx,&user).await?;
            let now=chrono::Utc::now().timestamp();let policy=policy(tx,&manifest,now).await?;
            let reader=load_reader(tx,&id,&user,&key_id).await?.ok_or(ApiError::NOT_FOUND)?;
            let (_,audiences)=authorize_reader(&reader,&key,&user,&manifest,policy.as_ref(),now)?;
            let rows=tx.query_all_raw(sql(r#"SELECT bundle FROM "DeviceFleetSnapshot" WHERE "deviceId"=$1 AND "userId"=$2 AND "keyId"=$3 ORDER BY "streamId" LIMIT 128"#,[id.into(),user.into(),key_id.into()])).await?;
            let mut snapshots=vec![];
            for row in rows {let bundle:EncryptedFleetSnapshot=serde_json::from_str(&row.try_get::<String>("","bundle")?)?;let snapshot=verify_fleet_snapshot(&bundle,&device.status.identity.telemetry_key).map_err(invalid)?;if audiences.contains(&snapshot.audience) {snapshots.push(bundle);}}
            Ok(FleetView {snapshots,policy_jws:policy.map(|(j,_)|j),reader_jws:reader.reader_jws})
        })
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_reader(issued_at: i64) -> String {
        let key = SigningKey::generate();
        let reader = FleetReader {
            version: 1,
            device_id: "device".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            user_id: "reader".into(),
            controller_key: key.public_key(),
            archive_key: [7; 32],
            revision: 1,
            issued_at,
            expires_at: issued_at + 86_400,
        };
        sign_fleet_reader(&reader, &key).unwrap()
    }

    #[test]
    fn a_recorded_acceptance_spaces_renewals_even_when_the_stored_declaration_is_backdated() {
        let now = 50_000;
        let backdated = signed_reader(now - 2 * MIN_READER_RENEWAL_SECONDS);
        assert!(check_renewal_interval(&backdated, None, now).is_ok());
        let refused = check_renewal_interval(&backdated, Some(now - 60), now).unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(
            check_renewal_interval(
                &backdated,
                Some(now - 60),
                now - 60 + MIN_READER_RENEWAL_SECONDS
            )
            .is_ok()
        );
    }

    #[test]
    fn declarations_signed_long_before_their_upload_are_accepted_once_the_interval_passed() {
        let now = 50_000;
        let stored = signed_reader(now - MIN_READER_RENEWAL_SECONDS);
        // A pending declaration retried after a failed upload, or signed by a
        // browser whose clock trails the server, carries an old issued_at.
        assert!(check_renewal_interval(&stored, None, now).is_ok());
        assert!(
            check_renewal_interval(&stored, Some(now - MIN_READER_RENEWAL_SECONDS - 1), now)
                .is_ok()
        );
    }

    #[test]
    fn reader_replacements_are_spaced_from_the_later_of_issue_and_acceptance() {
        let stored = signed_reader(10_000);
        assert_eq!(issued_at(&stored), Some(10_000));
        let refused = check_renewal_interval(
            &stored,
            Some(9_000),
            10_000 + MIN_READER_RENEWAL_SECONDS - 1,
        )
        .unwrap_err();
        assert_eq!(refused.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(
            refused
                .public_message()
                .is_some_and(|message| message.contains("13600"))
        );
        assert!(
            check_renewal_interval(&stored, Some(9_000), 10_000 + MIN_READER_RENEWAL_SECONDS)
                .is_ok()
        );
        assert!(check_renewal_interval("not-a-jws", None, 10_000).is_ok());
        assert!(check_renewal_interval("not-a-jws", Some(10_000), 10_001).is_err());
    }

    #[flow_like_types::tokio::test]
    #[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn fleet_latest_snapshot_reader_cas_revocation_and_lost_ack() {
        use sea_orm::{ConnectOptions, Database, TransactionTrait};
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").unwrap();
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("fleet_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        let mut scoped = reqwest::Url::parse(&url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let mut options = ConnectOptions::new(scoped.to_string());
        options.max_connections(6).min_connections(1);
        let db = Database::connect(options).await.unwrap();
        for migration in [
            include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
            include_str!("../../prisma/migrations/20260922120000_device_management/migration.sql"),
            include_str!("../../prisma/migrations/20260924150000_device_fleet/migration.sql"),
            include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
        ] {
            for statement in migration.split(';').filter(|s| !s.trim().is_empty()) {
                db.execute_unprepared(statement).await.unwrap();
            }
        }
        db.execute_unprepared(r#"CREATE TABLE "User"(id TEXT PRIMARY KEY,status TEXT NOT NULL); INSERT INTO "User" VALUES('owner','ACTIVE'),('reader','ACTIVE')"#).await.unwrap();
        let now = chrono::Utc::now().timestamp();
        let owner = SigningKey::generate();
        let invitation = SigningKey::generate();
        let reader = SigningKey::generate();
        let telemetry = SigningKey::generate();
        let identity = DeviceIdentity {
            auth_key: SigningKey::generate().public_key(),
            management_key: [42; 32],
            telemetry_key: telemetry.public_key(),
        };
        let receipt = DeviceReceipt {
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Fleet".into(),
            identity: identity.clone(),
            manifest_jws: "fixture".into(),
            binding_jws: "fixture".into(),
            registered_at: now,
            auth_epoch: 1,
        };
        db.execute_raw(sql(r#"INSERT INTO "ManagedDevice"(id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES('device','owner','Fleet','active',1,$1,$2,$3)"#,[serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),now.into()])).await.unwrap();
        let device = repository::current_device(&db, "device").await.unwrap();
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            name: "Fleet".into(),
            api_base_url: "https://hub.example/api/v1".into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: owner.public_key(),
            owner_invitation_key: invitation.public_key(),
            issued_at: now,
            expires_at: now + 600,
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            issued_at: now,
            expires_at: now + 600,
            grants: vec![ManagementGrant {
                grant_id: "grant".into(),
                user_id: "reader".into(),
                controller_key: reader.public_key(),
                scope: ManagementScope::Project {
                    project_id: "project".into(),
                },
                capabilities: vec![ManagementCapability::Status],
                expires_at: now + 300,
                group_id: None,
                group_version: None,
            }],
        };
        let compact = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',1,$1,$2,$3,$4)"#,[compact_digest(&compact).into(),compact.clone().into(),policy.expires_at.into(),now.into()])).await.unwrap();
        let key_id = URL_SAFE_NO_PAD.encode(reader.public_key().to_bytes().unwrap());
        let declaration = FleetReader {
            version: 1,
            device_id: "device".into(),
            api_base_url: manifest.api_base_url.clone(),
            user_id: "reader".into(),
            controller_key: reader.public_key(),
            archive_key: [7; 32],
            revision: 1,
            issued_at: now,
            expires_at: now + 600,
        };
        let signed = sign_fleet_reader(&declaration, &reader).unwrap();
        let aged = sign_fleet_reader(
            &FleetReader {
                issued_at: now - 2 * MIN_READER_RENEWAL_SECONDS,
                ..declaration.clone()
            },
            &reader,
        )
        .unwrap();
        // Stands in for the renewal interval passing since the stored reader was issued.
        let age_reader = || {
            db.execute_raw(sql(
                r#"UPDATE "DeviceFleetReader" SET "readerJws"=$1 WHERE "deviceId"='device' AND "userId"='reader' AND "keyId"=$2"#,
                [aged.clone().into(), key_id.clone().into()],
            ))
        };
        let reader_call_at = |user: &str, key_id: String, action, accepted_at| {
            retain_reader(
                &db,
                crate::db::DbDialect::Postgres,
                device.clone(),
                manifest.clone(),
                user.to_string(),
                key_id,
                accepted_at,
                action,
            )
        };
        let reader_call =
            |user: &str, key_id: String, action| reader_call_at(user, key_id, action, None);
        assert!(
            reader_call("reader", key_id.clone(), ReaderAction::Put(signed.clone()))
                .await
                .unwrap()
                .1
        );
        assert!(
            !reader_call("reader", key_id.clone(), ReaderAction::Put(signed.clone()))
                .await
                .unwrap()
                .1,
            "An acknowledged retry must not keep a new change record"
        );
        assert!(
            reader_call("owner", key_id.clone(), ReaderAction::Put(signed.clone()))
                .await
                .is_err()
        );
        let mut wrong = declaration.clone();
        wrong.api_base_url = "https://elsewhere.example/api/v1".into();
        assert!(
            reader_call(
                "reader",
                key_id.clone(),
                ReaderAction::Put(sign_fleet_reader(&wrong, &reader).unwrap())
            )
            .await
            .is_err()
        );
        let audience = fleet_audiences(
            &declaration,
            &signed,
            &manifest,
            Some((&compact, &policy)),
            now,
        )
        .remove(0);
        let make =
            |audience: FleetAudience, sequence, previous_digest: Option<String>, byte: u8| {
                let data = vec![byte; 32];
                let header = FleetManifest {
                    version: 1,
                    device_id: "device".into(),
                    api_base_url: manifest.api_base_url.clone(),
                    user_id: "reader".into(),
                    controller_key: reader.public_key(),
                    audience,
                    sequence,
                    previous_digest,
                    boot_id: "boot".into(),
                    observed_at: now,
                    encapsulation: [4; 32],
                    ciphertext_digest: fleet_digest(&data),
                    ciphertext_size: data.len() as u32,
                };
                EncryptedFleetSnapshot {
                    manifest_jws: sign_fleet_manifest(&header, &telemetry).unwrap(),
                    ciphertext: URL_SAFE_NO_PAD.encode(data),
                }
            };
        let first = make(audience.clone(), 1, None, 7);
        let upload = |bundle| {
            retain_snapshot(
                &db,
                crate::db::DbDialect::Postgres,
                device.clone(),
                manifest.clone(),
                bundle,
            )
        };
        let ack = upload(first.clone()).await.unwrap();
        assert_eq!(
            upload(first.clone()).await.unwrap().manifest_digest,
            ack.manifest_digest
        );
        assert!(upload(make(audience.clone(), 1, None, 8)).await.is_err());
        let read = || {
            retained_view(
                &db,
                crate::db::DbDialect::Postgres,
                device.clone(),
                manifest.clone(),
                "reader".into(),
                key_id.clone(),
            )
        };
        assert_eq!(read().await.unwrap().snapshots.len(), 1);
        let mut metrics = audience.clone();
        metrics.kind = FleetKind::Metrics;
        assert!(
            upload(make(metrics, 2, ack.manifest_digest.clone(), 9))
                .await
                .is_err()
        );
        db.execute_unprepared(r#"UPDATE "DeviceFleetSnapshot" SET "updatedAt"="updatedAt"-60"#)
            .await
            .unwrap();
        db.execute_unprepared(r#"UPDATE "DeviceFleetSnapshot" SET "manifestJws"='invalid'"#)
            .await
            .unwrap();
        assert!(
            upload(make(audience.clone(), 2, ack.manifest_digest.clone(), 9))
                .await
                .is_err()
        );
        db.execute_raw(sql(
            r#"UPDATE "DeviceFleetSnapshot" SET "manifestJws"=$1"#,
            [first.manifest_jws.clone().into()],
        ))
        .await
        .unwrap();
        let mut damaged = first.clone();
        damaged.ciphertext = URL_SAFE_NO_PAD.encode([8; 32]);
        db.execute_raw(sql(
            r#"UPDATE "DeviceFleetSnapshot" SET bundle=$1"#,
            [serde_json::to_string(&damaged).unwrap().into()],
        ))
        .await
        .unwrap();
        assert!(
            read().await.is_err(),
            "Download must authenticate the complete ciphertext"
        );
        // Pruning reads the small signed manifest only. Replacement can repair a
        // damaged payload without decoding every retained ciphertext.
        let second = make(audience.clone(), 2, ack.manifest_digest.clone(), 9);
        let second_digest = compact_digest(&second.manifest_jws);
        upload(second).await.unwrap();
        let current = read().await.unwrap();
        assert_eq!(current.snapshots.len(), 1);
        assert_eq!(
            compact_digest(&current.snapshots[0].manifest_jws),
            second_digest
        );
        let mut renewal = declaration.clone();
        renewal.revision = 2;
        renewal.issued_at = now - MIN_READER_RENEWAL_SECONDS - 1;
        let backdated = sign_fleet_reader(&renewal, &reader).unwrap();
        assert_eq!(
            reader_call(
                "reader",
                key_id.clone(),
                ReaderAction::Put(backdated.clone())
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "A backdated declaration must not skip the renewal interval"
        );
        age_reader().await.unwrap();
        assert_eq!(
            reader_call_at(
                "reader",
                key_id.clone(),
                ReaderAction::Put(backdated),
                Some(now)
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "A recorded acceptance must hold even when every signed issued_at is backdated"
        );
        renewal.issued_at = now;
        renewal.expires_at += 1;
        let renewal_a = sign_fleet_reader(&renewal, &reader).unwrap();
        renewal.expires_at += 1;
        let renewal_b = sign_fleet_reader(&renewal, &reader).unwrap();
        let (a, b) = tokio::join!(
            reader_call(
                "reader",
                key_id.clone(),
                ReaderAction::Put(renewal_a.clone())
            ),
            reader_call(
                "reader",
                key_id.clone(),
                ReaderAction::Put(renewal_b.clone())
            )
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let winner = if a.is_ok() { renewal_a } else { renewal_b };
        reader_call("reader", key_id.clone(), ReaderAction::Put(winner))
            .await
            .unwrap();
        assert!(read().await.unwrap().snapshots.is_empty());
        assert!(
            upload(make(audience.clone(), 3, Some(second_digest.clone()), 10))
                .await
                .is_err()
        );
        reader_call("reader", key_id.clone(), ReaderAction::Delete(3))
            .await
            .unwrap();
        reader_call("reader", key_id.clone(), ReaderAction::Delete(3))
            .await
            .unwrap();
        assert!(read().await.is_err());
        assert!(
            reader_call("reader", key_id.clone(), ReaderAction::Put(signed.clone()))
                .await
                .is_err()
        );
        let mut renewal = declaration.clone();
        renewal.revision = 4;
        // Signed before an upload that did not commit and retried as the pending declaration.
        renewal.issued_at = now - 2 * MIN_READER_RENEWAL_SECONDS;
        let renewal_signed = sign_fleet_reader(&renewal, &reader).unwrap();
        assert_eq!(
            reader_call(
                "reader",
                key_id.clone(),
                ReaderAction::Put(renewal_signed.clone()),
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Deleting and re-adding a reader must not skip the renewal interval"
        );
        age_reader().await.unwrap();
        reader_call(
            "reader",
            key_id.clone(),
            ReaderAction::Put(renewal_signed.clone()),
        )
        .await
        .unwrap();
        let latest_audience = fleet_audiences(
            &renewal,
            &renewal_signed,
            &manifest,
            Some((&compact, &policy)),
            now,
        )
        .remove(0);
        policy.grants.clear();
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&compact));
        let revoked = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',2,$1,$2,$3,$4)"#,[compact_digest(&revoked).into(),revoked.clone().into(),policy.expires_at.into(),now.into()])).await.unwrap();
        assert!(read().await.is_err());
        assert!(
            upload(make(latest_audience, 3, Some(second_digest), 11))
                .await
                .is_err()
        );
        let owner_key = URL_SAFE_NO_PAD.encode(owner.public_key().to_bytes().unwrap());
        let mut owner_declaration = FleetReader {
            user_id: "owner".into(),
            controller_key: owner.public_key(),
            issued_at: now,
            ..declaration.clone()
        };
        let owner_signed = sign_fleet_reader(&owner_declaration, &owner).unwrap();
        reader_call("owner", owner_key.clone(), ReaderAction::Put(owner_signed))
            .await
            .unwrap();
        // Every replacement invalidates the reader's in-flight publication, so
        // replacements within the renewal interval are refused.
        owner_declaration.revision = 2;
        assert_eq!(
            reader_call(
                "owner",
                owner_key.clone(),
                ReaderAction::Put(sign_fleet_reader(&owner_declaration, &owner).unwrap()),
            )
            .await
            .unwrap_err()
            .status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS
        );
        let grantee = SigningKey::generate();
        policy.grants = vec![ManagementGrant {
            grant_id: "grantee".into(),
            user_id: "grantee".into(),
            controller_key: grantee.public_key(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            capabilities: vec![ManagementCapability::Status],
            expires_at: now + 300,
            group_id: None,
            group_version: None,
        }];
        policy.policy_version = 3;
        policy.previous_policy_digest = Some(compact_digest(&revoked));
        let regranted = sign_management_policy(&policy, &invitation).unwrap();
        db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES('device',3,$1,$2,$3,$4)"#,[compact_digest(&regranted).into(),regranted.into(),policy.expires_at.into(),now.into()])).await.unwrap();
        db.execute_unprepared(
            r#"INSERT INTO "User" VALUES('grantee','ACTIVE'),('former','SUSPENDED')"#,
        )
        .await
        .unwrap();
        // Deleted rows, suspended accounts and keys the policy no longer grants
        // stay as history but must not use reader capacity.
        for index in 0..MAX_FLEET_READERS {
            let historical = SigningKey::generate();
            let user = if index % 2 == 0 { "former" } else { "reader" };
            let historical_jws = sign_fleet_reader(
                &FleetReader {
                    user_id: user.into(),
                    controller_key: historical.public_key(),
                    ..declaration.clone()
                },
                &historical,
            )
            .unwrap();
            db.execute_raw(sql(
                r#"INSERT INTO "DeviceFleetReader"("deviceId","userId","keyId",revision,"readerJws",deleted) VALUES('device',$1,$2,1,$3,$4)"#,
                [
                    user.into(),
                    URL_SAFE_NO_PAD
                        .encode(historical.public_key().to_bytes().unwrap())
                        .into(),
                    historical_jws.into(),
                    (index % 3 == 0).into(),
                ],
            ))
            .await
            .unwrap();
        }
        let grantee_key = URL_SAFE_NO_PAD.encode(grantee.public_key().to_bytes().unwrap());
        let grantee_signed = sign_fleet_reader(
            &FleetReader {
                user_id: "grantee".into(),
                controller_key: grantee.public_key(),
                ..declaration.clone()
            },
            &grantee,
        )
        .unwrap();
        reader_call(
            "grantee",
            grantee_key.clone(),
            ReaderAction::Put(grantee_signed),
        )
        .await
        .unwrap();
        let tx = db.begin().await.unwrap();
        let current = super::policy(&tx, &manifest, now).await.unwrap();
        let live: Vec<_> = live_readers(&tx, "device", &manifest, current.as_ref(), now)
            .await
            .unwrap()
            .into_iter()
            .map(|(user, key_id, _)| (user, key_id))
            .collect();
        tx.rollback().await.unwrap();
        assert_eq!(
            live,
            [
                ("grantee".to_string(), grantee_key),
                ("owner".to_string(), owner_key.clone())
            ]
        );
        db.execute_unprepared(
            r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id='device'"#,
        )
        .await
        .unwrap();
        assert!(
            reader_call("owner", owner_key, ReaderAction::Get)
                .await
                .is_err()
        );
        admin
            .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
    }
}
