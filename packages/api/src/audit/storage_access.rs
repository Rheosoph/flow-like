//! Correlate provider credential issuance with each authorization to use a grant.
//! A cached grant keeps its fingerprint, while every recipient gets a separate record.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
};

use chrono::{DateTime, Utc};
use flow_like::hub::AuditConfig;
use flow_like_device_protocol::InstanceStorageCredential;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::entity::sea_orm_active_enums::AuditActorType;
use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    error::ApiError,
    middleware::jwt::AppUser,
    state::State,
};

use super::{AuditRecordInput, WriteMode, actor_type_from_user, record, records, request};

#[cfg(feature = "aws")]
pub const CREDENTIAL_ISSUED: &str = "storage.credentials.issue";
pub const ACCESS_AUTHORIZED: &str = "file.access.authorize";

/// Hash the public access-key identifier, never the secret key or session token.
/// The same SHA-256 of a CloudTrail/GuardDuty accessKeyId locates its audit records.
pub(crate) fn access_key_fingerprint(access_key_id: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(access_key_id.as_bytes()))
    )
}

// Bound the encoded size, including JSON escapes. Long public identifiers remain
// searchable by their SHA-256 without displacing credential correlation fields.
fn identifier(value: &str) -> Value {
    if serde_json::to_vec(value)
        .expect("string serialization")
        .len()
        <= 160
    {
        json!(value)
    } else {
        json!({ "sha256": hex::encode(Sha256::digest(value.as_bytes())) })
    }
}

fn credential_fingerprints(credentials: &RuntimeCredentials) -> BTreeSet<String> {
    match credentials {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(credentials) => credentials
            .access_key_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(access_key_fingerprint)
            .into_iter()
            .collect(),
        RuntimeCredentials::Mixed(credentials) => {
            [&credentials.meta, &credentials.content, &credentials.logs]
                .into_iter()
                .flat_map(|credentials| credential_fingerprints(credentials))
                .collect()
        }
        #[allow(unreachable_patterns)]
        _ => BTreeSet::new(),
    }
}

fn credential_details(
    subject: &str,
    access: &CredentialsAccess,
    scope: &str,
    credentials: &RuntimeCredentials,
) -> Value {
    json!({
        "grant_kind": "credentials",
        "scope": scope,
        "subject": identifier(subject),
        "access_mode": access.to_string(),
        "expiration": credentials.expiration(),
        "credential_fingerprints": credential_fingerprints(credentials),
    })
}

pub(crate) struct InstanceStorageAuthorization<'a> {
    pub instance_id: &'a str,
    pub device_id: &'a str,
    pub grant_id: &'a str,
    pub subject: &'a str,
    pub app_id: &'a str,
    pub write: bool,
    pub expiration: i64,
    pub credentials: &'a BTreeMap<String, InstanceStorageCredential>,
}

fn instance_authorization_entry(grant: InstanceStorageAuthorization<'_>) -> AuditRecordInput {
    let fingerprints: BTreeSet<_> = grant
        .credentials
        .values()
        .filter_map(|credential| match credential {
            InstanceStorageCredential::AwsSession { access_key_id, .. } => {
                Some(access_key_fingerprint(access_key_id))
            }
            _ => None,
        })
        .collect();
    AuditRecordInput {
        actor_id: format!("instance:{}", grant.instance_id),
        actor_type: AuditActorType::System,
        actor_ip: request::actor_ip(),
        action: ACCESS_AUTHORIZED.to_owned(),
        resource_type: "Storage".to_owned(),
        resource_id: grant.app_id.to_owned(),
        scope: Some(grant.app_id.to_owned()),
        details: Some(json!({
            "grant_kind": "credentials",
            "scope": "instance_storage",
            "subject": identifier(grant.subject),
            "device_id": identifier(grant.device_id),
            "grant_id": identifier(grant.grant_id),
            "access_mode": if grant.write { "device_execute_write" } else { "device_execute_read" },
            "expiration": grant.expiration,
            "credential_fingerprints": fingerprints,
        })),
    }
}

/// Called only after the instance's original delegation has been rechecked.
pub(crate) async fn authorize_instance_storage(
    state: &State,
    grant: InstanceStorageAuthorization<'_>,
) -> Result<(), ApiError> {
    write_before_release(
        state,
        instance_authorization_entry(grant),
        WriteMode::Append,
    )
    .await
}

/// Called for each client response, including responses that reuse cached credentials.
/// The effective storage subject can differ from the authenticated actor.
pub(crate) async fn authorize_credentials(
    state: &State,
    user: &AppUser,
    app_id: &str,
    subject: &str,
    access: &CredentialsAccess,
    scope: &str,
    credentials: &RuntimeCredentials,
) -> Result<(), ApiError> {
    authorize(
        state,
        user,
        app_id,
        credential_details(subject, access, scope, credentials),
    )
    .await
}

/// Signed downloads are bearer grants too. Record their batch without URLs or filenames.
pub(crate) async fn authorize_downloads(
    state: &State,
    user: &AppUser,
    app_id: &str,
    scope: &str,
    destination_app_id: &str,
    fork_session_id: Option<&str>,
    count: usize,
    expiration: DateTime<Utc>,
) -> Result<(), ApiError> {
    authorize(
        state,
        user,
        app_id,
        download_details(
            scope,
            destination_app_id,
            fork_session_id,
            count,
            expiration,
        ),
    )
    .await
}

fn download_details(
    scope: &str,
    destination_app_id: &str,
    fork_session_id: Option<&str>,
    count: usize,
    expiration: DateTime<Utc>,
) -> Value {
    let mut details = json!({
        "grant_kind": "signed_downloads",
        "scope": scope,
        "access_mode": "read",
        "destination_app_id": identifier(destination_app_id),
        "file_count": count,
        "expiration": expiration,
    });
    // Content pages accept a destination seed from the caller. Only begin has
    // the server-created staging session; do not imply a verified session link.
    if let Some(session_id) = fork_session_id {
        details["fork_session_id"] = identifier(session_id);
    }
    details
}

async fn authorization_entry(
    user: &AppUser,
    app_id: &str,
    details: Value,
) -> Result<AuditRecordInput, ApiError> {
    let actor_id = match user {
        AppUser::Unauthorized => "anonymous".to_owned(),
        _ => user.audit_id().await.map_err(|_| {
            request::record_failure();
            ApiError::service_unavailable("Unable to identify storage access audit actor")
        })?,
    };
    Ok(AuditRecordInput {
        actor_id,
        actor_type: actor_type_from_user(user),
        actor_ip: request::actor_ip(),
        action: ACCESS_AUTHORIZED.to_owned(),
        resource_type: "Storage".to_owned(),
        resource_id: app_id.to_owned(),
        scope: Some(app_id.to_owned()),
        details: Some(details),
    })
}

async fn authorize(
    state: &State,
    user: &AppUser,
    app_id: &str,
    details: Value,
) -> Result<(), ApiError> {
    if !records(&state.platform_config.audit, ACCESS_AUTHORIZED) {
        return Ok(());
    }
    let entry = authorization_entry(user, app_id, details).await?;
    write_before_release(state, entry, WriteMode::Append).await
}

#[cfg(feature = "aws")]
pub(crate) struct AwsCredentialIssuance<'a> {
    pub subject: &'a str,
    pub app_id: &'a str,
    pub access: &'a CredentialsAccess,
    pub role_arn: &'a str,
    pub session_name: &'a str,
    pub request_id: Option<&'a str>,
    pub access_key_id: &'a str,
    pub policy_sha256: &'a str,
    pub expiration: DateTime<Utc>,
}

#[cfg(feature = "aws")]
fn issuance_entry(issuance: AwsCredentialIssuance<'_>) -> AuditRecordInput {
    let fingerprint = access_key_fingerprint(issuance.access_key_id);
    // This records the issuer and storage subject. The response record identifies
    // the authenticated recipient, including when another request reuses the grant.
    AuditRecordInput {
        actor_id: "aws.sts".to_owned(),
        actor_type: AuditActorType::System,
        actor_ip: request::actor_ip(),
        action: CREDENTIAL_ISSUED.to_owned(),
        resource_type: "StorageCredential".to_owned(),
        resource_id: fingerprint.clone(),
        scope: Some(issuance.app_id.to_owned()),
        details: Some(json!({
            "subject": identifier(issuance.subject),
            "access_mode": issuance.access.to_string(),
            "role_arn": identifier(issuance.role_arn),
            "session_name": issuance.session_name,
            "aws_request_id": issuance.request_id,
            "credential_fingerprint": fingerprint,
            "policy_sha256": issuance.policy_sha256,
            "expiration": issuance.expiration,
        })),
    }
}

/// Run only after a new STS session succeeds, before it can enter the cache or a response.
#[cfg(feature = "aws")]
pub(crate) async fn record_aws_issuance(
    state: &State,
    issuance: AwsCredentialIssuance<'_>,
) -> Result<(), ApiError> {
    write_before_release(state, issuance_entry(issuance), WriteMode::Once).await
}

async fn write_before_release(
    state: &State,
    input: AuditRecordInput,
    mode: WriteMode,
) -> Result<(), ApiError> {
    write_with(&state.platform_config.audit, input, |input| async move {
        record::write(&state.db, input, mode).await
    })
    .await
}

async fn write_with<F, Fut, E>(
    config: &AuditConfig,
    input: AuditRecordInput,
    write: F,
) -> Result<(), ApiError>
where
    F: FnOnce(AuditRecordInput) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    if !records(config, &input.action) {
        return Ok(());
    }
    if input.details.as_ref().is_some_and(|details| {
        super::crypto::canonical_json(details).len() > record::MAX_DETAILS_BYTES
    }) {
        request::record_failure();
        tracing::error!(
            action = input.action,
            "AUDIT FAILURE: storage grant details exceed the audit size limit"
        );
        return Err(ApiError::service_unavailable(
            "Storage access audit details exceed the size limit",
        ));
    }
    let action = input.action.clone();
    write(input).await.map_err(|error| {
        request::record_failure();
        tracing::error!(%error, action, "AUDIT FAILURE: storage grant was not released");
        ApiError::service_unavailable("Unable to persist storage access authorization")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditLevel, RetentionClass};
    #[cfg(feature = "aws")]
    use crate::middleware::jwt::OpenIDUser;
    use axum::http::StatusCode;
    use flow_like_types::tokio;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn audit_config(enabled: bool) -> AuditConfig {
        AuditConfig {
            enabled,
            level: AuditLevel::Standard,
            ..AuditConfig::default()
        }
    }

    #[tokio::test]
    async fn anonymous_downloads_keep_request_attribution_without_recording_urls() {
        let expiration = Utc::now() + chrono::Duration::minutes(30);
        let entry = request::REQUEST_AUDIT
            .scope(
                request::RequestAuditContext {
                    actor_ip: Some("192.0.2.7".to_owned()),
                    ..Default::default()
                },
                authorization_entry(
                    &AppUser::Unauthorized,
                    "public-app",
                    download_details("fork_content", "destination-123", None, 4, expiration),
                ),
            )
            .await
            .unwrap();
        assert_eq!(entry.actor_id, "anonymous");
        assert_eq!(entry.actor_ip.as_deref(), Some("192.0.2.7"));
        assert_eq!(entry.scope.as_deref(), Some("public-app"));
        assert_eq!(entry.chain_id(), "public-app");
        assert!(records(&audit_config(true), &entry.action));
        assert_eq!(RetentionClass::of(&entry.action), RetentionClass::Evidence);
        let details = entry.details.unwrap();
        assert_eq!(details["grant_kind"], "signed_downloads");
        assert_eq!(details["destination_app_id"], "destination-123");
        assert_eq!(details["file_count"], 4);
        assert_eq!(details["expiration"], json!(expiration));
        assert_eq!(
            details
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "grant_kind",
                "scope",
                "access_mode",
                "destination_app_id",
                "file_count",
                "expiration"
            ])
        );
        let begin = download_details(
            "fork_metadata",
            "destination-123",
            Some("session-456"),
            2,
            expiration,
        );
        assert_eq!(begin["destination_app_id"], details["destination_app_id"]);
        assert_eq!(begin["fork_session_id"], "session-456");
        assert!(details.get("fork_session_id").is_none());
    }

    #[tokio::test]
    async fn oversized_details_block_release_instead_of_losing_correlation() {
        let entry = authorization_entry(
            &AppUser::Unauthorized,
            "app",
            json!({"unbounded": "x".repeat(1024)}),
        )
        .await
        .unwrap();
        let called = AtomicBool::new(false);
        let error = write_with(&audit_config(true), entry, |_| async {
            called.store(true, Ordering::Relaxed);
            Ok::<_, &str>(())
        })
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!called.load(Ordering::Relaxed));
    }

    #[test]
    fn instance_storage_links_the_device_delegation_without_secrets() {
        let credentials = BTreeMap::from([(
            "device".to_owned(),
            InstanceStorageCredential::AwsSession {
                access_key_id: "ASIA_INSTANCE_PUBLIC_IDENTIFIER".to_owned(),
                secret_access_key: "instance-secret-must-not-enter-audit".to_owned(),
                session_token: "instance-token-must-not-enter-audit".to_owned(),
            },
        )]);
        let entry = instance_authorization_entry(InstanceStorageAuthorization {
            instance_id: "instance-123",
            device_id: "device-456",
            grant_id: "grant-789",
            subject: "delegating-user",
            app_id: "project-123",
            write: true,
            expiration: 1800000000,
            credentials: &credentials,
        });
        assert_eq!(entry.actor_id, "instance:instance-123");
        assert_eq!(entry.scope.as_deref(), Some("project-123"));
        let details = entry.details.as_ref().unwrap();
        assert_eq!(details["device_id"], "device-456");
        assert_eq!(details["grant_id"], "grant-789");
        assert_eq!(details["subject"], "delegating-user");
        assert_eq!(details["access_mode"], "device_execute_write");
        assert_eq!(
            details["credential_fingerprints"],
            json!([access_key_fingerprint("ASIA_INSTANCE_PUBLIC_IDENTIFIER")])
        );
        assert_eq!(details["expiration"], 1800000000);
        let serialized = serde_json::to_string(&entry).unwrap();
        for sensitive in [
            "ASIA_INSTANCE_PUBLIC_IDENTIFIER",
            "instance-secret-must-not-enter-audit",
            "instance-token-must-not-enter-audit",
        ] {
            assert!(!serialized.contains(sensitive));
        }
    }

    #[tokio::test]
    async fn audit_failure_blocks_a_grant_only_when_auditing_is_enabled() {
        let entry = authorization_entry(&AppUser::Unauthorized, "app", json!({}))
            .await
            .unwrap();
        let called = AtomicBool::new(false);
        write_with(&audit_config(false), entry.clone(), |_| async {
            called.store(true, Ordering::Relaxed);
            Err("audit store unavailable")
        })
        .await
        .unwrap();
        assert!(!called.load(Ordering::Relaxed));

        let context = request::RequestAuditContext::default();
        let failures = context.failures.clone();
        let result = request::REQUEST_AUDIT
            .scope(
                context,
                write_with(&audit_config(true), entry, |_| async {
                    Err("audit store unavailable")
                }),
            )
            .await;
        assert_eq!(
            result.unwrap_err().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(failures.load(Ordering::Relaxed), 1);
    }

    #[cfg(feature = "aws")]
    fn aws_credentials(key_id: &str) -> RuntimeCredentials {
        let mut credentials = crate::credentials::aws_credentials::AwsRuntimeCredentials::new(
            "meta",
            "content",
            "logs",
            "eu-west-1",
        );
        credentials.access_key_id = Some(key_id.to_owned());
        credentials.secret_access_key = Some("secret-must-not-enter-audit".to_owned());
        credentials.session_token = Some("token-must-not-enter-audit".to_owned());
        credentials.expiration = Some(Utc::now() + chrono::Duration::hours(1));
        RuntimeCredentials::Aws(credentials)
    }

    #[cfg(feature = "aws")]
    #[tokio::test]
    async fn cached_grant_responses_retain_each_actor_and_correlate_to_one_issuance() {
        let credentials = aws_credentials("ASIA_TEST_PUBLIC_IDENTIFIER");
        let access = CredentialsAccess::InvokeRead;
        let issued = issuance_entry(AwsCredentialIssuance {
            subject: "storage-subject",
            app_id: "app",
            access: &access,
            role_arn: "arn:aws:iam::123456789012:role/runtime-assumed",
            session_name: "storage-subject-app",
            request_id: Some("sts-request-123"),
            access_key_id: "ASIA_TEST_PUBLIC_IDENTIFIER",
            policy_sha256: "policy-digest",
            expiration: credentials.expiration().unwrap(),
        });
        let mut entries = vec![issued.clone()];
        for sub in ["first-recipient", "second-recipient"] {
            let user = AppUser::OpenID(OpenIDUser {
                sub: sub.to_owned(),
                access_token: "login-token-must-not-enter-audit".to_owned(),
            });
            let entry = authorization_entry(
                &user,
                "app",
                credential_details("storage-subject", &access, "invoke", &credentials),
            )
            .await
            .unwrap();
            write_with(&audit_config(true), entry, |entry| {
                entries.push(entry);
                async { Ok::<_, &str>(()) }
            })
            .await
            .unwrap();
        }
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].actor_id, "aws.sts");
        assert_eq!(entries[1].actor_id, "openid:first-recipient:");
        assert_eq!(entries[2].actor_id, "openid:second-recipient:");
        for entry in &entries[1..] {
            let details = entry.details.as_ref().unwrap();
            assert_eq!(details["subject"], "storage-subject");
            assert_eq!(details["access_mode"], "invoke_read");
            assert_eq!(
                details["credential_fingerprints"],
                json!([issued.resource_id])
            );
        }
        assert_eq!(
            issued.details.as_ref().unwrap()["aws_request_id"],
            "sts-request-123"
        );
        assert_eq!(issued.chain_id(), "app");
        assert!(records(&audit_config(true), &issued.action));
        for entry in &entries {
            assert!(serde_json::to_vec(&entry.details).unwrap().len() <= record::MAX_DETAILS_BYTES);
        }
        let serialized = serde_json::to_string(&entries).unwrap();
        for value in [
            "ASIA_TEST_PUBLIC_IDENTIFIER",
            "secret-must-not-enter-audit",
            "token-must-not-enter-audit",
            "login-token-must-not-enter-audit",
        ] {
            assert!(!serialized.contains(value));
        }
        let rotated = aws_credentials("ASIA_ROTATED_PUBLIC_IDENTIFIER");
        assert_ne!(
            credential_fingerprints(&credentials),
            credential_fingerprints(&rotated)
        );
    }

    #[cfg(feature = "aws")]
    #[test]
    fn mixed_credentials_deduplicate_reused_provider_grants() {
        let first = aws_credentials("ASIA_FIRST_IDENTIFIER");
        let second = aws_credentials("ASIA_SECOND_IDENTIFIER");
        let expected = BTreeSet::from([
            access_key_fingerprint("ASIA_FIRST_IDENTIFIER"),
            access_key_fingerprint("ASIA_SECOND_IDENTIFIER"),
        ]);
        let credentials = RuntimeCredentials::Mixed(
            crate::credentials::mixed_credentials::MixedRuntimeCredentials {
                meta: Box::new(first.clone()),
                content: Box::new(first),
                logs: Box::new(second),
            },
        );
        assert_eq!(credential_fingerprints(&credentials), expected);
    }

    #[cfg(feature = "aws")]
    #[test]
    fn long_identifiers_preserve_required_correlation_within_details_limit() {
        let subject = "u".repeat(128);
        let role_arn = format!("arn:aws:iam::123456789012:role/{}", "r".repeat(576));
        let session = "s".repeat(64);
        let policy = "a".repeat(64);
        let request_id = "00000000-0000-0000-0000-000000000000";
        let entry = issuance_entry(AwsCredentialIssuance {
            subject: &subject,
            app_id: "app",
            access: &CredentialsAccess::InvokeRead,
            role_arn: &role_arn,
            session_name: &session,
            request_id: Some(request_id),
            access_key_id: "ASIA_BOUNDARY_IDENTIFIER",
            policy_sha256: &policy,
            expiration: Utc::now(),
        });
        let details = entry.details.as_ref().unwrap();
        assert!(super::super::crypto::canonical_json(details).len() <= record::MAX_DETAILS_BYTES);
        assert_eq!(details["subject"], subject);
        assert_eq!(details["session_name"], session);
        assert_eq!(details["aws_request_id"], request_id);
        assert_eq!(details["policy_sha256"], policy);
        assert_eq!(details["credential_fingerprint"], entry.resource_id);
        assert_eq!(
            details["role_arn"]["sha256"],
            hex::encode(Sha256::digest(role_arn.as_bytes()))
        );

        let credentials = RuntimeCredentials::Mixed(
            crate::credentials::mixed_credentials::MixedRuntimeCredentials {
                meta: Box::new(aws_credentials("ASIA_META")),
                content: Box::new(aws_credentials("ASIA_CONTENT")),
                logs: Box::new(aws_credentials("ASIA_LOGS")),
            },
        );
        // Escapes expand JSON size even when the subject has only 128 characters.
        let escaped_subject = "\u{1}".repeat(128);
        let details = credential_details(
            &escaped_subject,
            &CredentialsAccess::InvokeRead,
            "invoke",
            &credentials,
        );
        assert!(super::super::crypto::canonical_json(&details).len() <= record::MAX_DETAILS_BYTES);
        assert_eq!(
            details["credential_fingerprints"].as_array().unwrap().len(),
            3
        );
        assert_eq!(
            details["subject"]["sha256"],
            hex::encode(Sha256::digest(escaped_subject.as_bytes()))
        );
    }
}
