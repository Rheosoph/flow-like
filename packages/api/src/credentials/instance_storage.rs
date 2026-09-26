use crate::{error::ApiError, state::AppState};
use flow_like_device_protocol::{
    InstanceStorageCredential, InstanceStorageLocation, OnlineProjectAccess, StoragePurpose,
    validate_instance_identifier,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct StorageIssueRequest {
    pub instance_id: String,
    pub project_id: String,
    pub delegating_user_id: String,
    pub access: OnlineProjectAccess,
    pub expires_at: i64,
}

impl StorageIssueRequest {
    pub fn validate(&self) -> Result<(), ApiError> {
        for id in [&self.instance_id, &self.project_id] {
            validate_instance_identifier(id)
                .map_err(|_| ApiError::bad_request("Invalid storage identity"))?;
        }
        super::device_execute_prefixes(&self.delegating_user_id, &self.project_id)
            .map_err(|_| ApiError::bad_request("Invalid storage scope"))?;
        super::device_execute_expiry(self.expires_at)
            .map_err(|_| ApiError::unauthorized("Storage authorization expired"))?;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct IssuedStorage {
    pub locations: BTreeMap<StoragePurpose, InstanceStorageLocation>,
    pub credentials: BTreeMap<String, InstanceStorageCredential>,
    pub expires_at: i64,
}

fn locations(
    scheme: &str,
    bucket: &str,
    request: &StorageIssueRequest,
    options: BTreeMap<String, String>,
    separate_credentials: bool,
) -> Result<BTreeMap<StoragePurpose, InstanceStorageLocation>, ApiError> {
    if bucket.is_empty()
        || !bucket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ApiError::not_implemented(
            "The content bucket cannot issue DeviceExecute credentials",
        ));
    }
    Ok(
        super::device_execute_prefixes(&request.delegating_user_id, &request.project_id)
            .map_err(|_| unavailable())?
            .into_iter()
            .map(|(purpose, prefix)| {
                (
                    purpose,
                    InstanceStorageLocation {
                        uri: format!("{scheme}://{bucket}/{prefix}"),
                        prefix,
                        credential_id: if separate_credentials {
                            purpose.as_str().into()
                        } else {
                            "device".into()
                        },
                        options: options.clone(),
                    },
                )
            })
            .collect(),
    )
}

fn unavailable() -> ApiError {
    ApiError::service_unavailable(
        "The storage provider could not issue bounded DeviceExecute credentials",
    )
}

fn provider_error(error: flow_like_types::Error) -> ApiError {
    let message = error.to_string();
    // Expose only our static configuration errors. Provider responses may contain
    // account information and never cross the instance boundary verbatim.
    if message.starts_with("STS session policy exceeds") {
        return ApiError::not_implemented(
            "DeviceExecute exceeds AWS's 2048-byte session policy limit; shorten the bucket, project, or user identifiers",
        );
    }
    if message.starts_with("DeviceExecute requires standard AWS S3") {
        return ApiError::not_implemented(
            "DeviceExecute requires standard AWS S3; custom STS endpoints and directory buckets are unsupported",
        );
    }
    if message.starts_with("GCP instance storage requires") {
        return ApiError::not_implemented(
            "DeviceExecute requires GCP_INSTANCE_STORAGE_SERVICE_ACCOUNT with short-lived impersonation permission",
        );
    }
    if message.starts_with("Instance storage requires a standard R2") {
        return ApiError::not_implemented(
            "DeviceExecute requires a standard R2 account endpoint and signing credential",
        );
    }
    unavailable()
}

pub(crate) async fn issue(
    state: &AppState,
    request: &StorageIssueRequest,
) -> Result<IssuedStorage, ApiError> {
    request.validate()?;
    let master = state
        .master_credentials()
        .await
        .map_err(|_| unavailable())?;
    let mut source = master.as_ref();
    while let super::RuntimeCredentials::Mixed(mixed) = source {
        source = &mixed.content;
    }
    let mode = super::CredentialsAccess::DeviceExecute {
        write: request.access == OnlineProjectAccess::ReadWrite,
        expires_at: request.expires_at,
    };
    let scoped = super::mixed_credentials::scope_inner(
        source,
        &request.delegating_user_id,
        &request.project_id,
        state,
        mode,
    )
    .await
    .map_err(provider_error)?;
    let expires_at = scoped.expiration().ok_or_else(unavailable)?.timestamp();
    let (locations, credentials) = match scoped {
        #[cfg(feature = "aws")]
        super::RuntimeCredentials::Aws(c) => (
            locations(
                "s3",
                &c.content_bucket,
                request,
                c.device_storage_options(),
                false,
            )?,
            BTreeMap::from([(
                "device".into(),
                InstanceStorageCredential::AwsSession {
                    access_key_id: c.access_key_id.ok_or_else(unavailable)?,
                    secret_access_key: c.secret_access_key.ok_or_else(unavailable)?,
                    session_token: c.session_token.ok_or_else(unavailable)?,
                },
            )]),
        ),
        #[cfg(feature = "r2")]
        super::RuntimeCredentials::R2(c) => (
            locations(
                "s3",
                &c.content_bucket,
                request,
                BTreeMap::from([
                    ("aws_region".into(), "auto".into()),
                    ("aws_endpoint".into(), c.endpoint.clone()),
                    ("aws_virtual_hosted_style_request".into(), "false".into()),
                ]),
                false,
            )?,
            BTreeMap::from([(
                "device".into(),
                InstanceStorageCredential::AwsSession {
                    access_key_id: c.access_key_id.ok_or_else(unavailable)?,
                    secret_access_key: c.secret_access_key.ok_or_else(unavailable)?,
                    session_token: c.session_token.ok_or_else(unavailable)?,
                },
            )]),
        ),
        #[cfg(feature = "azure")]
        super::RuntimeCredentials::Azure(c) => (
            locations(
                "az",
                &c.content_container,
                request,
                BTreeMap::from([("azure_storage_account_name".into(), c.account_name.clone())]),
                true,
            )?,
            c.device_sas_tokens
                .into_iter()
                .map(|(key, sas_token)| (key, InstanceStorageCredential::AzureSas { sas_token }))
                .collect(),
        ),
        #[cfg(feature = "gcp")]
        super::RuntimeCredentials::Gcp(c) => (
            locations("gs", &c.content_bucket, request, BTreeMap::new(), false)?,
            BTreeMap::from([(
                "device".into(),
                InstanceStorageCredential::GcpBearer {
                    access_token: c.access_token.ok_or_else(unavailable)?,
                },
            )]),
        ),
        super::RuntimeCredentials::Mixed(_) => unreachable!("content provider is unwrapped"),
    };
    request.validate()?;
    if expires_at > request.expires_at
        || expires_at <= chrono::Utc::now().timestamp()
        || locations
            .values()
            .any(|l| !credentials.contains_key(&l.credential_id))
    {
        return Err(unavailable());
    }
    Ok(IssuedStorage {
        locations,
        credentials,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_scope_matches_user_paths_and_excludes_server_only_prefixes() {
        let request = StorageIssueRequest {
            instance_id: "instance".into(),
            project_id: "project".into(),
            delegating_user_id: "auth0|owner".into(),
            access: OnlineProjectAccess::ReadOnly,
            expires_at: chrono::Utc::now().timestamp() + 3590,
        };
        request.validate().unwrap();
        let scoped = locations("s3", "content", &request, BTreeMap::new(), false).unwrap();
        assert_eq!(scoped.len(), 4);
        assert_eq!(
            scoped[&StoragePurpose::Files].prefix,
            "apps/project/upload/"
        );
        assert_eq!(
            scoped[&StoragePurpose::Storage].prefix,
            "apps/project/storage/"
        );
        assert_eq!(
            scoped[&StoragePurpose::User].prefix,
            "users/auth0%7Cowner/apps/project/"
        );
        assert_eq!(
            scoped[&StoragePurpose::Temporary].prefix,
            format!(
                "{}/",
                super::super::temporary_prefixes("auth0|owner", "project").0
            )
        );
        for location in scoped.values() {
            assert_eq!(location.credential_id, "device");
            assert_eq!(location.uri, format!("s3://content/{}", location.prefix));
            for excluded in [
                "metadata/",
                "media/",
                "tmp/global/",
                "runs/",
                "users/other/",
            ] {
                assert!(!location.prefix.contains(excluded));
            }
        }
        for (field, value) in [("project", "project/other"), ("user", "owner/other")] {
            let mut invalid = request.clone();
            if field == "project" {
                invalid.project_id = value.into();
            } else {
                invalid.delegating_user_id = value.into();
            }
            assert!(invalid.validate().is_err());
        }
        let mut expired = request.clone();
        expired.expires_at = chrono::Utc::now().timestamp() - 1;
        assert!(expired.validate().is_err());
        expired.expires_at = chrono::Utc::now().timestamp() + 3601;
        assert!(expired.validate().is_err());
    }
}
