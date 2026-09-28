use super::{Connection, auth, client, config::ManagedConfig, endpoint, guid, store};
use crate::{error::ApiError, state::AppState};
use flow_like_types::tokio;
use serde_json::{Value, json};
use std::time::Duration;

type Provisioner = ManagedConfig;

const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";
const ARM_SCOPE: &str = "https://management.azure.com/.default";
const PURGE_ATTEMPTS: u64 = 3;

impl Provisioner {
    async fn required(state: &AppState) -> Result<Self, ApiError> {
        Self::load(&state.secrets).await?.ok_or_else(|| {
            ApiError::bad_request("Flow-Like-managed bots are not enabled on this server. Choose a customer-managed bot or ask your administrator.")
        })
    }

    async fn token(&self, scope: &str) -> Result<String, ApiError> {
        Ok(
            auth::client_token(&self.tenant, &self.id, &self.secret, scope)
                .await?
                .0,
        )
    }
}

async fn call(
    method: reqwest::Method,
    url: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let mut request = client()?.request(method, url).bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|_| {
        ApiError::bad_gateway("Microsoft bot provisioning could not be reached. Retry setup.")
    })?;
    let status = response.status();
    if super::transient(status) {
        return Err(ApiError::bad_gateway(format!(
            "Microsoft bot provisioning returned HTTP {status}. Retry shortly."
        )));
    }
    if !status.is_success() {
        // Microsoft errors can echo submitted credentials. Only expose the status.
        return Err(ApiError::bad_request(format!(
            "Microsoft bot provisioning returned HTTP {status}. Check the provisioning identity's Graph permissions and Azure resource-group access, then retry."
        )));
    }
    if status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    response
        .json()
        .await
        .map_err(|_| ApiError::internal("Invalid Microsoft provisioning response"))
}

async fn persist(state: &AppState, c: &Connection, revision: &mut i32) -> Result<(), ApiError> {
    store::save_connection(state, c, *revision).await?;
    *revision += 1;
    Ok(())
}

pub(super) async fn provision(
    state: &AppState,
    c: &mut Connection,
    revision: &mut i32,
) -> Result<(), ApiError> {
    let p = Provisioner::required(state).await?;
    c.home_tenant_id = p.tenant.clone();
    let graph = p.token(GRAPH_SCOPE).await?;
    if c.graph_object_id.is_none() {
        // Recover an application created before a crash interrupted local persistence.
        let app = match tagged_application(&graph, &c.id).await? {
            Some(app) => app,
            None => {
                call(reqwest::Method::POST,"https://graph.microsoft.com/v1.0/applications",&graph,Some(json!({
                    "displayName":c.name,
                    "signInAudience":"AzureADMultipleOrgs",
                    "owners@odata.bind":[format!("https://graph.microsoft.com/v1.0/directoryObjects/{}",p.owner)],
                    "tags":[recovery_tag(&c.id)]
                }))).await?
            }
        };
        c.graph_object_id = Some(guid(app["id"].as_str().ok_or_else(|| {
            ApiError::internal("Microsoft did not return an application ID")
        })?)?);
        c.client_id = guid(
            app["appId"]
                .as_str()
                .ok_or_else(|| ApiError::internal("Microsoft did not return a client ID"))?,
        )?;
        persist(state, c, revision).await?;
    }
    let object = c.graph_object_id.clone().ok_or(ApiError::NOT_FOUND)?;
    cleanup_pending(state, c, revision, &graph, &object).await?;
    // Creation and retries share the same application. Graph may need time to replicate it.
    let principals = call(
        reqwest::Method::GET,
        &format!(
            "https://graph.microsoft.com/v1.0/servicePrincipals?$filter=appId%20eq%20'{}'",
            c.client_id
        ),
        &graph,
        None,
    )
    .await?;
    if principals["value"].as_array().is_none_or(Vec::is_empty) {
        call(reqwest::Method::POST,"https://graph.microsoft.com/v1.0/servicePrincipals",&graph,Some(json!({
            "appId":c.client_id,
            "owners@odata.bind":[format!("https://graph.microsoft.com/v1.0/directoryObjects/{}",p.owner)]
        }))).await?;
    }
    if c.secret.is_empty() {
        add_secret(state, c, revision, &graph, &object).await?;
    }
    let resource = format!(
        "/subscriptions/{}/resourceGroups/{}/providers/Microsoft.BotService/botServices/fl-{}",
        p.subscription, p.group, c.id
    );
    c.azure_resource_id = Some(resource.clone());
    persist(state, c, revision).await?;
    let arm = p.token(ARM_SCOPE).await?;
    put_bot(state, c, &resource, &arm).await?;
    call(reqwest::Method::PUT,&format!("https://management.azure.com{resource}/channels/MsTeamsChannel?api-version=2022-09-15"),&arm,Some(json!({
        "location":"global","properties":{"channelName":"MsTeamsChannel","properties":{"isEnabled":true}}
    }))).await?;
    auth::client_token(&c.home_tenant_id, &c.client_id, &c.secret, auth::BOT_SCOPE).await?;
    Ok(())
}

async fn put_bot(
    state: &AppState,
    c: &Connection,
    resource: &str,
    arm: &str,
) -> Result<(), ApiError> {
    call(reqwest::Method::PUT,&format!("https://management.azure.com{resource}?api-version=2022-09-15"),arm,Some(json!({
        "location":"global","kind":"azurebot","sku":{"name":"F0"},
        "properties":{"displayName":c.name,"description":c.description,"endpoint":endpoint(state, &c.id).await?,
        "msaAppId":c.client_id,"msaAppType":"SingleTenant","msaAppTenantId":c.home_tenant_id}
    }))).await?;
    Ok(())
}

/// Applies a new display name or description to a provisioned bot without touching its credentials.
pub(super) async fn update_bot(state: &AppState, c: &Connection) -> Result<(), ApiError> {
    let resource = c
        .azure_resource_id
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("Complete bot setup before editing it"))?;
    let arm = Provisioner::required(state).await?.token(ARM_SCOPE).await?;
    put_bot(state, c, resource, &arm).await
}

pub(super) fn recovery_tag(connection: &str) -> String {
    format!("flow-like-teams:{connection}")
}

async fn tagged_application(graph: &str, connection: &str) -> Result<Option<Value>, ApiError> {
    let mut lookup = reqwest::Url::parse("https://graph.microsoft.com/v1.0/applications")
        .map_err(|_| ApiError::internal("Invalid Microsoft Graph URL"))?;
    lookup.query_pairs_mut().extend_pairs([
        (
            "$filter",
            format!("tags/any(tag:tag eq '{}')", recovery_tag(connection)),
        ),
        ("$select", "id,appId".into()),
        ("$top", "2".into()),
    ]);
    let found = call(reqwest::Method::GET, lookup.as_str(), graph, None).await?;
    recovered_application(&found)
}

fn recovered_application(response: &Value) -> Result<Option<Value>, ApiError> {
    let applications = response["value"]
        .as_array()
        .ok_or_else(|| ApiError::internal("Invalid Microsoft application lookup response"))?;
    if applications.len() > 1 || response.get("@odata.nextLink").is_some() {
        return Err(ApiError::conflict(
            "Several Microsoft applications have this bot's recovery tag. Ask your administrator to resolve the duplicate registrations before retrying.",
        ));
    }
    Ok(applications.first().cloned())
}

async fn add_secret(
    state: &AppState,
    c: &mut Connection,
    revision: &mut i32,
    graph: &str,
    object: &str,
) -> Result<(), ApiError> {
    let expires = (chrono::Utc::now() + chrono::Duration::days(180)).to_rfc3339();
    let secret = call(
        reqwest::Method::POST,
        &format!("https://graph.microsoft.com/v1.0/applications/{object}/addPassword"),
        graph,
        Some(json!({
            "passwordCredential":{"displayName":"Flow-Like Teams bot","endDateTime":expires}
        })),
    )
    .await?;
    let mut candidate = match replacement_credential(c, &secret, expires) {
        Ok(candidate) => candidate,
        Err(error) => {
            if let Some(key) = secret["keyId"]
                .as_str()
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
            {
                queue_secret(c, &key.to_string());
                let _ = persist(state, c, revision).await;
                let _ = cleanup_pending(state, c, revision, graph, object).await;
            }
            return Err(error);
        }
    };
    let key = candidate
        .secret_key_id
        .clone()
        .ok_or_else(|| ApiError::internal("Microsoft did not return a bot credential ID"))?;
    // Until promotion, recovery may delete this credential while retaining the
    // active secret. Promotion atomically replaces this queue entry with the old key.
    queue_secret(c, &key);
    if let Err(error) = persist(state, c, revision).await {
        if remove_secret(graph, object, &key).await.is_ok() {
            c.pending_secret_key_ids.retain(|pending| pending != &key);
        }
        return Err(error);
    }
    // First setup verifies the credential after registering the Azure Bot.
    // Rotation must verify the replacement while the active secret still works.
    if !c.secret.is_empty()
        && let Err(error) = auth::client_token(
            &candidate.home_tenant_id,
            &candidate.client_id,
            &candidate.secret,
            auth::BOT_SCOPE,
        )
        .await
    {
        let _ = cleanup_pending(state, c, revision, graph, object).await;
        return Err(error);
    }

    candidate.pending_secret_key_ids = c.pending_secret_key_ids.clone();
    candidate
        .pending_secret_key_ids
        .retain(|pending| pending != &key);
    if let Some(old) = &c.secret_key_id {
        queue_secret(&mut candidate, old);
    }

    if let Err(error) = persist(state, &candidate, revision).await {
        // A lost database response can follow a successful write. Never revoke
        // the candidate until we know it is not the stored active credential.
        match store::connection(state, &c.id).await {
            Ok(Some((stored, stored_revision)))
                if stored.secret_key_id == candidate.secret_key_id =>
            {
                *c = stored;
                *revision = stored_revision;
                return Ok(());
            }
            Ok(Some((stored, stored_revision))) => {
                *c = stored;
                *revision = stored_revision;
                let _ = cleanup_pending(state, c, revision, graph, object).await;
            }
            Ok(None) => {
                if remove_secret(graph, object, &key).await.is_ok() {
                    c.pending_secret_key_ids.retain(|pending| pending != &key);
                }
            }
            Err(_) => {
                tracing::warn!(connection_id = %c.id, "Could not confirm credential persistence; retaining the candidate credential");
            }
        }
        return Err(error);
    }
    *c = candidate;
    Ok(())
}

fn replacement_credential(
    current: &Connection,
    response: &Value,
    expires: String,
) -> Result<Connection, ApiError> {
    let secret = response["secretText"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("Microsoft did not return a bot secret"))?;
    let key = response["keyId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("Microsoft did not return a bot credential ID"))?;
    let key = uuid::Uuid::parse_str(key)
        .map_err(|_| ApiError::internal("Microsoft returned an invalid bot credential ID"))?;
    let mut candidate = current.clone();
    candidate.secret = secret.into();
    candidate.secret_key_id = Some(key.to_string());
    candidate.secret_expires_at = Some(expires);
    Ok(candidate)
}

fn queue_secret(c: &mut Connection, key: &str) {
    if !c
        .secret_key_id
        .as_deref()
        .is_some_and(|active| active.eq_ignore_ascii_case(key))
        && !c
            .pending_secret_key_ids
            .iter()
            .any(|pending| pending.eq_ignore_ascii_case(key))
    {
        c.pending_secret_key_ids.push(key.into());
    }
}

async fn cleanup_pending(
    state: &AppState,
    c: &mut Connection,
    revision: &mut i32,
    graph: &str,
    object: &str,
) -> Result<(), ApiError> {
    if c.pending_secret_key_ids.is_empty() {
        return Ok(());
    }
    let (stored, _) = store::connection(state, &c.id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if stored.secret_key_id != c.secret_key_id {
        return Err(ApiError::conflict(
            "The active Teams credential changed. Retry cleanup.",
        ));
    }
    // A previous deletion may have succeeded before its database update failed.
    let application = call(
        reqwest::Method::GET,
        &format!(
            "https://graph.microsoft.com/v1.0/applications/{object}?$select=passwordCredentials"
        ),
        graph,
        None,
    )
    .await?;
    let mut remaining = Vec::new();
    for key in cleanup_keys(c, &application)? {
        if remove_secret(graph, object, &key).await.is_err() {
            remaining.push(key);
        }
    }
    let mut updated = c.clone();
    updated.pending_secret_key_ids = remaining;
    persist(state, &updated, revision).await?;
    *c = updated;
    if !c.pending_secret_key_ids.is_empty() {
        return Err(ApiError::bad_request(
            "Microsoft could not remove an unused bot credential. Retry setup or rotation to finish cleanup.",
        ));
    }
    Ok(())
}

fn cleanup_keys(c: &Connection, application: &Value) -> Result<Vec<String>, ApiError> {
    let remote_keys = application["passwordCredentials"]
        .as_array()
        .ok_or_else(|| ApiError::internal("Invalid Microsoft credential list"))?;
    Ok(c.pending_secret_key_ids
        .iter()
        .filter(|key| {
            !c.secret_key_id
                .as_deref()
                .is_some_and(|active| active.eq_ignore_ascii_case(key))
                && remote_keys.iter().any(|remote| {
                    remote["keyId"]
                        .as_str()
                        .is_some_and(|remote| remote.eq_ignore_ascii_case(key))
                })
        })
        .cloned()
        .collect())
}

async fn remove_secret(graph: &str, object: &str, key: &str) -> Result<(), ApiError> {
    call(
        reqwest::Method::POST,
        &format!("https://graph.microsoft.com/v1.0/applications/{object}/removePassword"),
        graph,
        Some(json!({"keyId":key})),
    )
    .await?;
    Ok(())
}

pub(super) async fn rotate(
    state: &AppState,
    c: &mut Connection,
    revision: &mut i32,
) -> Result<(), ApiError> {
    let p = Provisioner::required(state).await?;
    let graph = p.token(GRAPH_SCOPE).await?;
    let object = c
        .graph_object_id
        .clone()
        .ok_or_else(|| ApiError::bad_request("Complete bot setup before rotating credentials"))?;
    cleanup_pending(state, c, revision, &graph, &object).await?;
    add_secret(state, c, revision, &graph, &object).await?;
    cleanup_pending(state, c, revision, &graph, &object).await
}

/// Deletes a managed bot's Azure Bot and Entra application. With `recover`, an application
/// whose creation was never persisted is found through its recovery tag.
pub(super) async fn remove(
    state: &AppState,
    c: &Connection,
    recover: bool,
) -> Result<(), ApiError> {
    let recorded = c.azure_resource_id.is_some() || c.graph_object_id.is_some();
    if !recorded && !recover {
        return Ok(());
    }
    let Some(p) = Provisioner::load(&state.secrets).await? else {
        if recorded {
            return Err(ApiError::bad_request(
                "Flow-Like-managed bots are disabled on this server, so this bot's Microsoft registration cannot be removed. Ask your administrator to re-enable managed provisioning, then retry.",
            ));
        }
        return Ok(());
    };
    if let Some(resource) = &c.azure_resource_id {
        let arm = p.token(ARM_SCOPE).await?;
        delete(
            &format!("https://management.azure.com{resource}?api-version=2022-09-15"),
            &arm,
            "Azure Bot registration",
        )
        .await?;
    }
    let graph = p.token(GRAPH_SCOPE).await?;
    if let Some(object) = application_to_remove(&graph, c).await? {
        delete(
            &format!("https://graph.microsoft.com/v1.0/applications/{object}"),
            &graph,
            "bot identity",
        )
        .await?;
        purge_deleted_application(&graph, &object).await;
    }
    Ok(())
}

async fn application_to_remove(graph: &str, c: &Connection) -> Result<Option<String>, ApiError> {
    match &c.graph_object_id {
        Some(object) => Ok(Some(object.clone())),
        None => tagged_application(graph, &c.id)
            .await?
            .map(|app| guid(app["id"].as_str().unwrap_or_default()))
            .transpose(),
    }
}

async fn delete(url: &str, token: &str, resource: &str) -> Result<(), ApiError> {
    let status = client()?
        .delete(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| {
            ApiError::bad_gateway(format!(
                "Microsoft could not be reached to remove the {resource}"
            ))
        })?
        .status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(if super::transient(status) {
        ApiError::bad_gateway(format!(
            "Microsoft returned HTTP {status} while removing the {resource}. Retry shortly."
        ))
    } else {
        ApiError::bad_request(format!(
            "Microsoft refused to remove the {resource} (HTTP {status})"
        ))
    })
}

/// A deleted application stays in the directory's deleted items, and counts against its
/// quota, for 30 days. Purging is best effort: Microsoft removes it after that period anyway.
async fn purge_deleted_application(graph: &str, object: &str) {
    let url = format!("https://graph.microsoft.com/v1.0/directory/deletedItems/{object}");
    for attempt in 1..=PURGE_ATTEMPTS {
        match delete_status(&url, graph).await {
            Some(status) if status.is_success() => return,
            Some(status) if purge_refused(status) => {
                tracing::warn!(graph_object_id = %object, %status, "Microsoft refused to purge the removed Teams bot application");
                return;
            }
            _ if attempt < PURGE_ATTEMPTS => {
                tokio::time::sleep(Duration::from_secs(attempt)).await;
            }
            _ => {}
        }
    }
    tracing::warn!(graph_object_id = %object, "Could not purge the removed Teams bot application; Microsoft deletes it after 30 days");
}

async fn delete_status(url: &str, token: &str) -> Option<reqwest::StatusCode> {
    let response = client().ok()?.delete(url).bearer_auth(token).send().await;
    response.ok().map(|response| response.status())
}

/// The deleted item can take a moment to appear, so only other client errors are final.
fn purge_refused(status: reqwest::StatusCode) -> bool {
    status.is_client_error()
        && status != reqwest::StatusCode::NOT_FOUND
        && !super::transient(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_connection() -> Connection {
        Connection {
            id: "connection".into(),
            app_id: "app".into(),
            event_id: "event".into(),
            mode: super::super::AuthMode::FlowLikeManaged,
            name: "Bot".into(),
            description: String::new(),
            customer_tenant_id: "customer-tenant".into(),
            home_tenant_id: "home-tenant".into(),
            client_id: "client".into(),
            secret: "active-secret".into(),
            graph_object_id: Some("graph-application".into()),
            azure_resource_id: Some("azure-bot".into()),
            secret_key_id: Some("11111111-1111-1111-1111-111111111111".into()),
            secret_expires_at: Some("2027-01-01T00:00:00Z".into()),
            pending_secret_key_ids: vec![],
            status: "ready".into(),
            allowed_responders: vec!["approver".into()],
            operation_until: 123,
        }
    }

    #[test]
    fn malformed_password_responses_cannot_become_active_credentials() {
        let current = active_connection();
        for response in [
            json!({"secretText":"replacement"}),
            json!({"secretText":"replacement","keyId":""}),
            json!({"secretText":"replacement","keyId":"invalid"}),
            json!({"secretText":"","keyId":"22222222-2222-2222-2222-222222222222"}),
            json!({"keyId":"22222222-2222-2222-2222-222222222222"}),
        ] {
            assert!(replacement_credential(&current, &response, "expiry".into()).is_err());
        }
    }

    #[test]
    fn preparing_replacement_keeps_the_active_credential_and_bot_identity() {
        let current = active_connection();
        let original = serde_json::to_value(&current).unwrap();
        let candidate = replacement_credential(
            &current,
            &json!({"secretText":"replacement","keyId":"22222222-2222-2222-2222-222222222222"}),
            "2027-06-01T00:00:00Z".into(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&current).unwrap(), original);
        let mut expected = original;
        expected["secret"] = json!("replacement");
        expected["secret_key_id"] = json!("22222222-2222-2222-2222-222222222222");
        expected["secret_expires_at"] = json!("2027-06-01T00:00:00Z");
        assert_eq!(serde_json::to_value(candidate).unwrap(), expected);
    }

    #[test]
    fn recovery_refuses_duplicate_or_malformed_application_results() {
        assert!(recovered_application(&json!({})).is_err());
        assert!(recovered_application(&json!({"value":[{},{}]})).is_err());
        assert!(recovered_application(&json!({"value":[{}],"@odata.nextLink":"next"})).is_err());
        assert!(
            recovered_application(&json!({"value":[]}))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            recovered_application(&json!({"value":[{"id":"existing","appId":"client"}]})).unwrap(),
            Some(json!({"id":"existing","appId":"client"}))
        );
    }

    #[test]
    fn cleanup_queue_never_adds_the_active_key_or_duplicate_keys() {
        let mut current = active_connection();
        current.secret_key_id = Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into());
        queue_secret(&mut current, "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA");
        queue_secret(&mut current, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        queue_secret(&mut current, "BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB");
        assert_eq!(
            current.pending_secret_key_ids,
            vec!["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"]
        );
    }

    #[test]
    fn cleanup_recovery_preserves_active_and_unrelated_credentials() {
        let mut current = active_connection();
        current.secret_key_id = Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into());
        current.pending_secret_key_ids = vec![
            "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA".into(),
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            "cccccccc-cccc-cccc-cccc-cccccccccccc".into(),
        ];
        let application = json!({"passwordCredentials":[
            {"keyId":"AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA"},
            {"keyId":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"},
            {"keyId":"dddddddd-dddd-dddd-dddd-dddddddddddd"}
        ]});
        assert_eq!(
            cleanup_keys(&current, &application).unwrap(),
            vec!["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"]
        );
        assert!(cleanup_keys(&current, &json!({})).is_err());
    }
}
