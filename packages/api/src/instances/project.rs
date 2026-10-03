use super::*;
use crate::{
    credentials::instance_storage::{StorageIssueRequest, issue},
    permission::role_permission::{RolePermissions, has_role_permission},
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
};

const JOSE_TYPE: &str = "flow-like-instance-project+jwt";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectClaims {
    pub sub: String,
    pub act: jwt::Actor,
    pub instance_id: String,
    pub device_id: String,
    pub device_auth_epoch: u64,
    pub key_epoch: u64,
    pub grant_id: String,
    pub authz_version: u64,
    pub deployment_id: String,
    pub placement_id: String,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "InstancePurpose::is_workload")]
    pub purpose: InstancePurpose,
    pub access: OnlineProjectAccess,
    pub cnf: devices::jwt::Confirmation,
    pub dpop_nonce: String,
    pub scope: String,
    pub typ: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

fn scope(access: OnlineProjectAccess, purpose: InstancePurpose) -> &'static str {
    if purpose == InstancePurpose::RolloutValidation {
        return INSTANCE_PROJECT_METADATA_SCOPE;
    }
    match access {
        OnlineProjectAccess::ReadOnly => INSTANCE_PROJECT_READ_SCOPE,
        OnlineProjectAccess::ReadWrite => INSTANCE_PROJECT_WRITE_SCOPE,
    }
}

fn verify(token: &str) -> Result<ProjectClaims, ApiError> {
    let c: ProjectClaims = backend_jwt::verify_typed(token, TokenType::InstanceProject, JOSE_TYPE)
        .map_err(bad_proof)?;
    if c.typ != TokenType::InstanceProject
        || c.scope != scope(c.access, c.purpose)
        || (c.purpose == InstancePurpose::RolloutValidation
            && c.access != OnlineProjectAccess::ReadOnly)
        || c.act.sub != format!("instance:{}", c.instance_id)
        || c.sub.is_empty()
        || c.cnf.jkt.is_empty()
        || c.dpop_nonce.len() != 43
        || c.device_auth_epoch == 0
        || c.key_epoch == 0
        || c.authz_version == 0
        || c.iat <= 0
        || c.iat > now() + 5
        || c.nbf != c.iat
        || c.exp <= c.iat
        || c.exp - c.iat > MAX_INSTANCE_PROJECT_TOKEN_SECONDS
        || c.jti.is_empty()
    {
        return Err(bad_proof("project token claims are malformed"));
    }
    for id in [
        &c.instance_id,
        &c.device_id,
        &c.grant_id,
        &c.deployment_id,
        &c.placement_id,
        &c.project_id,
    ] {
        validate_instance_identifier(id).map_err(bad_proof)?;
    }
    proof_live(c.exp)?;
    Ok(c)
}

pub(crate) async fn token(
    state: &DeviceContext<'_>,
    id: &str,
    request: InstanceTokenRequest,
) -> Result<InstanceTokenResponse, ApiError> {
    let graph = authenticated_instance(
        state,
        id,
        &request.client_assertion,
        &format!("/instances/{id}/project-token"),
        true,
    )
    .await?;
    let grant = &graph.grant.info;
    let granted_access = grant.online_access.ok_or(ApiError::FORBIDDEN)?;
    let access = if graph.instance.receipt.purpose == InstancePurpose::RolloutValidation {
        OnlineProjectAccess::ReadOnly
    } else {
        granted_access
    };
    if grant.app_id.as_deref() != Some(&grant.project_id) {
        return Err(ApiError::FORBIDDEN);
    }
    let receipt = &graph.instance.receipt;
    let timestamp = now();
    let expires_at =
        (timestamp + MAX_INSTANCE_PROJECT_TOKEN_SECONDS).min(resource_deadline(&graph)?);
    live(expires_at)?;
    let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let claims = ProjectClaims {
        sub: grant.delegating_user_id.clone(),
        act: jwt::Actor {
            sub: format!("instance:{id}"),
        },
        instance_id: id.into(),
        device_id: receipt.device_id.clone(),
        device_auth_epoch: graph.device.status.auth_epoch,
        key_epoch: receipt.key_epoch,
        grant_id: grant.grant_id.clone(),
        authz_version: grant.authz_version,
        deployment_id: grant.deployment_id.clone(),
        placement_id: grant.placement_id.clone(),
        project_id: grant.project_id.clone(),
        purpose: receipt.purpose,
        access,
        cnf: devices::jwt::Confirmation {
            jkt: receipt.workload_key.thumbprint().map_err(bad_proof)?,
        },
        dpop_nonce: nonce.clone(),
        scope: scope(access, receipt.purpose).into(),
        typ: TokenType::InstanceProject,
        iss: backend_jwt::issuer().into(),
        aud: INSTANCE_PROJECT_AUDIENCE.into(),
        iat: timestamp,
        nbf: timestamp,
        exp: expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    Ok(InstanceTokenResponse {
        access_token: backend_jwt::sign_typed(&claims, JOSE_TYPE)
            .map_err(|_| ApiError::internal("Cannot sign project token"))?,
        token_type: "DPoP".into(),
        expires_in: (expires_at - timestamp) as u64,
        expires_at,
        dpop_nonce: nonce,
        lease_expires_at: receipt.lease_expires_at,
    })
}

fn validate(graph: &Graph, c: &ProjectClaims, deadline: i64) -> Result<(), ApiError> {
    let g = &graph.grant.info;
    let i = &graph.instance.receipt;
    let access = if i.purpose == InstancePurpose::RolloutValidation {
        g.online_access.map(|_| OnlineProjectAccess::ReadOnly)
    } else {
        g.online_access
    };
    if access != Some(c.access)
        || i.purpose != c.purpose
        || g.app_id.as_deref() != Some(c.project_id.as_str())
        || g.project_id != c.project_id
        || g.delegating_user_id != c.sub
        || i.instance_id != c.instance_id
        || i.device_id != c.device_id
        || graph.device.status.auth_epoch != c.device_auth_epoch
        || i.key_epoch != c.key_epoch
        || i.workload_key.thumbprint().map_err(bad_proof)? != c.cnf.jkt
        || g.grant_id != c.grant_id
        || g.authz_version != c.authz_version
        || g.placement_id != c.placement_id
        || g.deployment_id != c.deployment_id
    {
        return Err(ApiError::FORBIDDEN);
    }
    // Expired authority is a denial. An expired proof or project token only needs a fresh one.
    live(resource_deadline(graph)?)?;
    proof_live(deadline.min(c.exp))
}

#[derive(Clone)]
pub(super) struct AuthorizedProject {
    pub(super) claims: ProjectClaims,
    deadline: i64,
}

pub(super) async fn audit(
    state: &AppState,
    claims: &ProjectClaims,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    details: serde_json::Value,
) {
    crate::audit::record_entry(
        state,
        audit_record(claims, action, resource_type, resource_id, details),
    )
    .await;
}

/// The record names the instance and the delegation so the project owner can trace
/// device credentials and writes.
pub(super) fn audit_record(
    claims: &ProjectClaims,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    details: serde_json::Value,
) -> crate::audit::AuditRecordInput {
    delegation_audit::instance_record(
        &claims.instance_id,
        Some(&claims.project_id),
        action,
        resource_type,
        resource_id,
        delegation_details(claims, details),
    )
}

fn delegation_details(claims: &ProjectClaims, mut details: serde_json::Value) -> serde_json::Value {
    if let Some(object) = details.as_object_mut() {
        object.insert("device_id".into(), claims.device_id.clone().into());
        object.insert("grant_id".into(), claims.grant_id.clone().into());
        object.insert("delegated_user".into(), claims.sub.clone().into());
    }
    details
}

pub(super) async fn recheck_in(
    tx: &sea_orm::DatabaseTransaction,
    authorization: &AuthorizedProject,
) -> Result<(), ApiError> {
    let graph = lock_graph(tx, &authorization.claims.instance_id, false).await?;
    validate(&graph, &authorization.claims, authorization.deadline)
}

fn validation_metadata_request(method: &str, path: &str) -> bool {
    if method != "GET" {
        return false;
    }
    if path == "/instances/project/app" {
        return true;
    }
    let Some(path) = path.strip_prefix("/instances/project/") else {
        return false;
    };
    let parts = path.split('/').collect::<Vec<_>>();
    (parts.len() == 6
        || (parts.len() == 8
            && parts[0] == "boards"
            && parts[6] == "pages"
            && validate_instance_identifier(parts[7]).is_ok()))
        && matches!(parts[0], "boards" | "events" | "widgets" | "templates")
        && validate_instance_identifier(parts[1]).is_ok()
        && parts[2] == "versions"
        && parts[3..6].iter().all(|part| {
            part.parse::<u32>()
                .is_ok_and(|v| v < u32::MAX && v.to_string() == *part)
        })
}

pub(super) async fn authenticate(
    state: &DeviceContext<'_>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
) -> Result<AuthorizedProject, ApiError> {
    devices::enabled(state)?;
    let (token, proof) = credentials(headers)?;
    let claims = verify(token)?;
    if claims.purpose == InstancePurpose::RolloutValidation
        && !validation_metadata_request(method, path)
    {
        return Err(ApiError::forbidden(
            "Validation instances can only read project metadata",
        ));
    }
    for name in [
        "x-flow-like-payer-id",
        "x-flow-like-user-id",
        "x-flow-like-instance-id",
        "x-flow-like-app-id",
    ] {
        if headers.contains_key(name) {
            return Err(ApiError::forbidden(
                "Project attribution is fixed by the instance grant",
            ));
        }
    }
    let instance = read_instance(state.db, &claims.instance_id).await?;
    let proof = verify_dpop(
        proof,
        &instance.receipt.workload_key,
        &DpopContext {
            method,
            url: &endpoint(state, path)?,
            access_token: Some(token),
            nonce: Some(&claims.dpop_nonce),
            key_thumbprint: &claims.cnf.jkt,
            now: now(),
        },
    )
    .map_err(bad_proof)?;
    let authorization = AuthorizedProject {
        deadline: (proof.iat + MAX_ASSERTION_TTL_SECONDS).min(claims.exp),
        claims,
    };
    let result = authorization.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let a = authorization.clone();
            let proof = proof.clone();
            Box::pin(async move {
                let graph = lock_graph(tx, &a.claims.instance_id, false).await?;
                validate(&graph, &a.claims, a.deadline)?;
                consume_instance_proof(
                    tx,
                    &a.claims.instance_id,
                    a.claims.key_epoch,
                    &proof.jti,
                    a.deadline,
                )
                .await?;
                validate(&graph, &a.claims, a.deadline)
            })
        },
    )
    .await?;
    Ok(result)
}

pub(super) async fn recheck(
    state: &DeviceContext<'_>,
    authorization: &AuthorizedProject,
) -> Result<i64, ApiError> {
    devices::enabled(state)?;
    let a = authorization.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let a = a.clone();
            Box::pin(async move {
                let graph = lock_graph(tx, &a.claims.instance_id, false).await?;
                validate(&graph, &a.claims, a.deadline)?;
                Ok(a.claims.exp.min(resource_deadline(&graph)?))
            })
        },
    )
    .await
}

pub(super) async fn recheck_storage(
    state: &DeviceContext<'_>,
    authorization: &AuthorizedProject,
) -> Result<i64, ApiError> {
    devices::enabled(state)?;
    let a = authorization.clone();
    retry_transaction(
        state.db,
        state.dialect,
        None,
        &RetryPolicy::default(),
        move |tx| {
            let a = a.clone();
            Box::pin(async move {
                let graph = lock_graph(tx, &a.claims.instance_id, false).await?;
                validate(&graph, &a.claims, a.deadline)?;
                if a.claims.purpose != InstancePurpose::Workload {
                    return Err(ApiError::FORBIDDEN);
                }
                // Direct cloud access deliberately survives the registration lease.
                // Revocation prevents refresh; already issued credentials expire in at most one hour.
                device_deployment_deadline(tx, &graph.device, &graph.grant.info).await
            })
        },
    )
    .await
}

/// A full storage quota narrows a ReadWrite lease to reads: placements keep serving
/// existing data while the provider credential refuses every write.
fn quota_access(quota: Result<(), ApiError>) -> Result<OnlineProjectAccess, ApiError> {
    match quota {
        Ok(()) => Ok(OnlineProjectAccess::ReadWrite),
        Err(error) if error.status() == axum::http::StatusCode::PAYMENT_REQUIRED => {
            Ok(OnlineProjectAccess::ReadOnly)
        }
        Err(error) => Err(error),
    }
}

/// How approval lists report that narrowing. A quota that cannot be read right now is
/// unknown, not full.
fn write_block(quota: Result<(), ApiError>) -> Option<OnlineWriteBlock> {
    matches!(quota_access(quota), Ok(OnlineProjectAccess::ReadOnly))
        .then_some(OnlineWriteBlock::StorageFull)
}

/// The projects among `project_ids` that `viewer` can read as a member.
async fn readable_projects<C: ConnectionTrait>(
    db: &C,
    viewer: &str,
    project_ids: &[String],
) -> Result<HashSet<String>, ApiError> {
    if project_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = db
        .query_all_raw(sql(
            &format!(
                r#"SELECT m."appId",r.permissions FROM "Membership" m JOIN "Role" r ON r.id=m."roleId" AND r."appId"=m."appId" JOIN "App" a ON a.id=m."appId" WHERE m."userId"=$1 AND a.status='ACTIVE' AND m."appId" IN ({})"#,
                placeholders(2, project_ids.len())
            ),
            std::iter::once(viewer.into()).chain(project_ids.iter().map(|id| id.clone().into())),
        ))
        .await?;
    let mut readable = HashSet::new();
    for row in rows {
        let permissions = RolePermissions::from_bits(row.try_get::<i64>("", "permissions")?);
        if permissions.is_some_and(|permissions| {
            has_role_permission(&permissions, RolePermissions::ReadBoards)
        }) {
            readable.insert(row.try_get("", "appId")?);
        }
    }
    Ok(readable)
}

/// Marks the read-and-write approvals whose next lease `storage` would narrow to reads.
/// Only the approver and people who can read the project learn its storage state: a device
/// owner outside the project must not.
pub(crate) async fn mark_blocked_writes(
    state: &AppState,
    viewer: &str,
    grants: &mut [ResourceGrantResponse],
) -> Result<(), ApiError> {
    mark_blocked_writes_with(&state.db, viewer, grants, |project_id, payer| async move {
        crate::capacity::check_storage_write(state, &project_id, &payer, 0).await
    })
    .await
}

/// `quota` answers for a project and its fallback payer, once per pair.
pub(super) async fn mark_blocked_writes_with<C, Q, F>(
    db: &C,
    viewer: &str,
    grants: &mut [ResourceGrantResponse],
    quota: Q,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
    Q: Fn(String, String) -> F,
    F: Future<Output = Result<(), ApiError>>,
{
    let now = now();
    let writes = |grant: &ResourceGrantResponse| {
        grant.status == "active"
            && grant.online_access == Some(OnlineProjectAccess::ReadWrite)
            && grant.expires_at > now
    };
    let mut foreign: Vec<String> = grants
        .iter()
        .filter(|grant| writes(grant) && grant.delegating_user_id != viewer)
        .map(|grant| grant.project_id.clone())
        .collect();
    foreign.sort();
    foreign.dedup();
    let readable = readable_projects(db, viewer, &foreign).await?;
    let mut checked = HashMap::new();
    for grant in grants.iter_mut().filter(|grant| writes(grant)) {
        if grant.delegating_user_id != viewer && !readable.contains(&grant.project_id) {
            continue;
        }
        let project = (grant.project_id.clone(), grant.delegating_user_id.clone());
        grant.online_write_blocked = match checked.get(&project) {
            Some(block) => *block,
            None => {
                let block = write_block(quota(project.0.clone(), project.1.clone()).await);
                checked.insert(project, block);
                block
            }
        };
    }
    Ok(())
}

pub(crate) async fn storage(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<InstanceStorageLease, ApiError> {
    let context = devices::context(state);
    let a = authenticate(&context, headers, "POST", "/instances/project/storage").await?;
    let access = match a.claims.access {
        OnlineProjectAccess::ReadWrite => quota_access(
            crate::capacity::check_storage_write(state, &a.claims.project_id, &a.claims.sub, 0)
                .await,
        )?,
        OnlineProjectAccess::ReadOnly => OnlineProjectAccess::ReadOnly,
    };
    if access != a.claims.access {
        tracing::info!(
            instance_id = %a.claims.instance_id,
            project_id = %a.claims.project_id,
            "Storage quota is full; issuing a read-only instance storage lease"
        );
    }
    let deadline = recheck_storage(&context, &a).await?;
    let request = StorageIssueRequest {
        instance_id: a.claims.instance_id.clone(),
        project_id: a.claims.project_id.clone(),
        delegating_user_id: a.claims.sub.clone(),
        access,
        expires_at: (now() + MAX_INSTANCE_STORAGE_LEASE_SECONDS).min(deadline),
    };
    let issued = issue(state, &request).await?;
    // Provider calls can outlive a revoke. Do not return their credentials until
    // the exact original authorization has survived a second transaction.
    let deadline = recheck_storage(&context, &a).await?;
    if issued.expires_at > deadline {
        // The grant is still valid but its deadline moved; a retry is bounded by the new one.
        return Err(ApiError::service_unavailable(
            "Storage authorization changed while credentials were issued; retry the lease",
        ));
    }
    audit(
        state,
        &a.claims,
        "instance.storage.lease",
        "PlacementResourceGrant",
        &a.claims.grant_id,
        serde_json::json!({ "access": access, "expires_at": issued.expires_at }),
    )
    .await;
    let c = a.claims;
    Ok(InstanceStorageLease {
        instance_id: c.instance_id,
        device_id: c.device_id,
        device_auth_epoch: c.device_auth_epoch,
        key_epoch: c.key_epoch,
        grant_id: c.grant_id,
        authz_version: c.authz_version,
        project_id: c.project_id,
        placement_id: c.placement_id,
        deployment_id: c.deployment_id,
        delegating_user_id: c.sub,
        access,
        expires_at: issued.expires_at,
        grant_expires_at: Some(deadline),
        locations: issued.locations,
        credentials: issued.credentials,
    })
}

pub(crate) async fn artifact(
    state: &AppState,
    headers: &HeaderMap,
    kind: &str,
    id: &str,
    version: (u32, u32, u32),
) -> Result<serde_json::Value, ApiError> {
    validate_instance_identifier(id).map_err(invalid)?;
    if !matches!(kind, "boards" | "events" | "widgets" | "templates") {
        return Err(ApiError::NOT_FOUND);
    }
    let path = format!(
        "/instances/project/{kind}/{id}/versions/{}/{}/{}",
        version.0, version.1, version.2
    );
    let context = devices::context(state);
    let a = authenticate(&context, headers, "GET", &path).await?;
    let c = &a.claims;
    let value = match kind {
        "templates" => {
            let mut template = flow_like::flow::board::Board::load_template(
                flow_like_storage::Path::from("apps").join(c.project_id.as_str()),
                id,
                state.master_state(state).await?,
                Some(version),
            )
            .await?;
            crate::routes::app::board::secrets::filter_board_secrets(&mut template);
            serde_json::to_value(template)?
        }
        "boards" => {
            let mut board = state
                .master_board(&c.sub, &c.project_id, id, state, Some(version))
                .await?;
            let builtin = state.registry.as_ref().get_nodes_shared();
            let wasm =
                crate::routes::app::wasm_catalog::app_wasm_nodes_cached(state, &c.project_id)
                    .await?;
            crate::routes::app::wasm_catalog::hydrate_board_wasm_metadata(
                &mut board,
                &wasm.nodes,
                &builtin,
            );
            crate::routes::app::board::secrets::filter_board_secrets(&mut board);
            serde_json::to_value(board)?
        }
        "events" => {
            let app = state.master_app(&c.sub, &c.project_id, state).await?;
            let event = flow_like::flow::event::Event::load_pinned(id, &app, version).await?;
            serde_json::to_value(crate::routes::app::device_metadata::device_event(event)?)?
        }
        "widgets" => {
            let app = state.master_app(&c.sub, &c.project_id, state).await?;
            serde_json::to_value(app.open_widget(id.into(), Some(version)).await?)?
        }
        _ => unreachable!(),
    };
    recheck(&context, &a).await?;
    Ok(value)
}

pub(crate) async fn app(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<serde_json::Value, ApiError> {
    let context = devices::context(state);
    let a = authenticate(&context, headers, "GET", "/instances/project/app").await?;
    let mut app = state
        .master_app(&a.claims.sub, &a.claims.project_id, state)
        .await?;
    state.hydrate_app_visibility(&mut app).await?;
    // Keep an explicit wire allowlist. Future runtime or credential fields on
    // App must not become public merely because its serializer changes.
    let value = serde_json::json!({"id":app.id,"status":app.status,"visibility":app.visibility,
        "authors":app.authors,"bits":app.bits,"boards":app.boards,"events":app.events,"templates":app.templates,
        "changelog":app.changelog,"primary_category":app.primary_category,"secondary_category":app.secondary_category,
        "app_type":app.app_type,"rating_sum":app.rating_sum,"rating_count":app.rating_count,
        "download_count":app.download_count,"interactions_count":app.interactions_count,"avg_rating":app.avg_rating,
        "relevance_score":app.relevance_score,"execution_mode":app.execution_mode,"updated_at":app.updated_at,"created_at":app.created_at,
        "widget_ids":app.widget_ids,"page_ids":app.page_ids,"packages":app.packages,"frontend":app.frontend,
        "version":app.version,"price":app.price,"allow_forking":app.allow_forking,"forked_from":app.forked_from,"forked_at":app.forked_at});
    recheck(&context, &a).await?;
    Ok(value)
}

pub(crate) async fn page(
    state: &AppState,
    headers: &HeaderMap,
    board_id: &str,
    page_id: &str,
    version: (u32, u32, u32),
) -> Result<serde_json::Value, ApiError> {
    validate_instance_identifier(board_id).map_err(invalid)?;
    validate_instance_identifier(page_id).map_err(invalid)?;
    if [version.0, version.1, version.2].contains(&u32::MAX) {
        return Err(ApiError::bad_request("Page version must be concrete"));
    }
    let path = format!(
        "/instances/project/boards/{board_id}/versions/{}/{}/{}/pages/{page_id}",
        version.0, version.1, version.2
    );
    let context = devices::context(state);
    let authorization = authenticate(&context, headers, "GET", &path).await?;
    let claims = &authorization.claims;
    let board = state
        .master_board(
            &claims.sub,
            &claims.project_id,
            board_id,
            state,
            Some(version),
        )
        .await?;
    if board.id != board_id
        || board.version != version
        || !board.page_ids.iter().any(|id| id == page_id)
    {
        return Err(ApiError::NOT_FOUND);
    }
    let page = board.load_versioned_page(page_id, version, None).await?;
    if page.id != page_id || page.board_id.as_deref().is_some_and(|id| id != board_id) {
        return Err(ApiError::NOT_FOUND);
    }
    recheck(&context, &authorization).await?;
    Ok(serde_json::to_value(page)?)
}

#[cfg(test)]
pub(super) fn test_claims() -> ProjectClaims {
    let timestamp = now();
    ProjectClaims {
        sub: "owner".into(),
        act: jwt::Actor {
            sub: "instance:instance".into(),
        },
        instance_id: "instance".into(),
        device_id: "device".into(),
        device_auth_epoch: 1,
        key_epoch: 1,
        grant_id: "grant".into(),
        authz_version: 1,
        deployment_id: "deployment".into(),
        placement_id: "placement".into(),
        project_id: "project".into(),
        purpose: InstancePurpose::Workload,
        access: OnlineProjectAccess::ReadOnly,
        cnf: devices::jwt::Confirmation { jkt: "key".into() },
        dpop_nonce: "a".repeat(43),
        scope: INSTANCE_PROJECT_READ_SCOPE.into(),
        typ: TokenType::InstanceProject,
        iss: backend_jwt::issuer().into(),
        aud: INSTANCE_PROJECT_AUDIENCE.into(),
        iat: timestamp,
        nbf: timestamp,
        exp: timestamp + 300,
        jti: "jti".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_token_profile_has_no_model_user_or_device_fallback() {
        backend_jwt::init_for_tests();
        let claims = test_claims();
        let timestamp = claims.iat;
        let token = backend_jwt::sign_typed(&claims, JOSE_TYPE).unwrap();
        assert!(verify(&token).is_ok());
        assert!(jwt::verify(&token).is_err());
        assert!(devices::jwt::is_device_credential(&token));
        assert!(devices::jwt::verify_session(&token).is_err());
        for (field, value) in [
            ("scope", serde_json::json!("models:invoke")),
            ("exp", serde_json::json!(timestamp + 301)),
            ("act", serde_json::json!({"sub":"owner"})),
            ("project_id", serde_json::json!("../other")),
            ("billing_grant_id", serde_json::json!("injected")),
            ("aud", serde_json::json!(INSTANCE_MODELS_AUDIENCE)),
        ] {
            let mut changed = serde_json::to_value(&claims).unwrap();
            changed[field] = value;
            assert!(
                verify(&backend_jwt::sign_typed(&changed, JOSE_TYPE).unwrap()).is_err(),
                "accepted {field}"
            );
        }
        let mut validation = claims.clone();
        validation.purpose = InstancePurpose::RolloutValidation;
        validation.scope = INSTANCE_PROJECT_METADATA_SCOPE.into();
        assert!(verify(&backend_jwt::sign_typed(&validation, JOSE_TYPE).unwrap()).is_ok());
        validation.access = OnlineProjectAccess::ReadWrite;
        assert!(verify(&backend_jwt::sign_typed(&validation, JOSE_TYPE).unwrap()).is_err());
        validation.access = OnlineProjectAccess::ReadOnly;
        validation.scope = INSTANCE_PROJECT_READ_SCOPE.into();
        assert!(verify(&backend_jwt::sign_typed(&validation, JOSE_TYPE).unwrap()).is_err());
    }

    #[test]
    fn malformed_or_expired_project_tokens_are_proof_failures() {
        backend_jwt::init_for_tests();
        let timestamp = now();
        let claims = serde_json::json!({
            "sub": "owner", "act": {"sub": "instance:other"}, "instance_id": "instance",
            "device_id": "device", "device_auth_epoch": 1, "key_epoch": 1, "grant_id": "grant",
            "authz_version": 1, "deployment_id": "deployment", "placement_id": "placement",
            "project_id": "project", "access": OnlineProjectAccess::ReadOnly,
            "cnf": {"jkt": "key"}, "dpop_nonce": "a".repeat(43),
            "scope": INSTANCE_PROJECT_READ_SCOPE, "typ": TokenType::InstanceProject,
            "iss": backend_jwt::issuer(), "aud": INSTANCE_PROJECT_AUDIENCE,
            "iat": timestamp, "nbf": timestamp, "exp": timestamp + 300, "jti": "jti",
        });
        let error = verify(&backend_jwt::sign_typed(&claims, JOSE_TYPE).unwrap()).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(error.public_code(), INSTANCE_PROOF_INVALID);
        assert_eq!(
            verify("not-a-token").unwrap_err().public_code(),
            INSTANCE_PROOF_INVALID
        );
    }

    #[test]
    fn full_storage_quota_narrows_the_lease_to_reads() {
        assert_eq!(
            quota_access(Ok(())).unwrap(),
            OnlineProjectAccess::ReadWrite
        );
        let full = ApiError::coded(
            axum::http::StatusCode::PAYMENT_REQUIRED,
            "PLAN_LIMIT_EXCEEDED",
            "Storage is full",
        );
        assert_eq!(
            quota_access(Err(full)).unwrap(),
            OnlineProjectAccess::ReadOnly
        );
        let outage = quota_access(Err(ApiError::internal("capacity lookup failed")));
        assert_eq!(outage.unwrap_err().status().as_u16(), 500);
    }

    #[test]
    fn approval_lists_report_a_full_quota_and_never_guess() {
        let full = ApiError::coded(
            axum::http::StatusCode::PAYMENT_REQUIRED,
            "PLAN_LIMIT_EXCEEDED",
            "Storage is full",
        );
        assert_eq!(write_block(Err(full)), Some(OnlineWriteBlock::StorageFull));
        assert_eq!(write_block(Ok(())), None);
        assert_eq!(
            write_block(Err(ApiError::internal("capacity lookup failed"))),
            None
        );
    }

    #[test]
    fn instance_audit_details_name_the_delegation() {
        let timestamp = now();
        let claims = ProjectClaims {
            sub: "owner".into(),
            act: jwt::Actor {
                sub: "instance:instance".into(),
            },
            instance_id: "instance".into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
            grant_id: "grant".into(),
            authz_version: 1,
            deployment_id: "deployment".into(),
            placement_id: "placement".into(),
            project_id: "project".into(),
            purpose: InstancePurpose::Workload,
            access: OnlineProjectAccess::ReadWrite,
            cnf: devices::jwt::Confirmation { jkt: "key".into() },
            dpop_nonce: "a".repeat(43),
            scope: INSTANCE_PROJECT_WRITE_SCOPE.into(),
            typ: TokenType::InstanceProject,
            iss: "issuer".into(),
            aud: INSTANCE_PROJECT_AUDIENCE.into(),
            iat: timestamp,
            nbf: timestamp,
            exp: timestamp + 300,
            jti: "jti".into(),
        };
        assert_eq!(
            delegation_details(&claims, serde_json::json!({"access": "read_only"})),
            serde_json::json!({
                "access": "read_only",
                "device_id": "device",
                "grant_id": "grant",
                "delegated_user": "owner",
            })
        );
    }

    #[test]
    fn validation_allows_only_concrete_metadata_reads() {
        for path in [
            "/instances/project/app",
            "/instances/project/events/event/versions/1/0/0",
            "/instances/project/boards/board/versions/1/0/0",
            "/instances/project/widgets/widget/versions/1/0/0",
            "/instances/project/templates/template/versions/1/0/0",
            "/instances/project/boards/board/versions/1/0/0/pages/page",
        ] {
            assert!(validation_metadata_request("GET", path));
            assert!(!validation_metadata_request("POST", path));
        }
        for path in [
            "/instances/project/storage/files",
            "/instances/project/storage/metadata",
            "/instances/project/boards/b/versions/4294967295/0/0",
            "/instances/project/boards/b/versions/01/0/0",
            "/instances/project/boards/b/versions/1/0/0/pages/..",
            "/instances/project/events/e/versions/1/0/0/pages/p",
            "/instances/project/app?project=other",
            "/instances/chat/completions",
        ] {
            assert!(!validation_metadata_request("GET", path));
        }
    }
}
