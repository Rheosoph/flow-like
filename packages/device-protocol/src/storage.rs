//! Online resources use a separate audience and immutable, provider-enforced paths.

use crate::OnlineProjectAccess;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const INSTANCE_PROJECT_AUDIENCE: &str = "flow-like-project-resources";
pub const INSTANCE_PROJECT_READ_SCOPE: &str = "project:read storage:read";
pub const INSTANCE_PROJECT_WRITE_SCOPE: &str = "project:read storage:read storage:write";
pub const MAX_INSTANCE_STORAGE_LEASE_SECONDS: i64 = 3600;
pub const MAX_INSTANCE_PROJECT_TOKEN_SECONDS: i64 = 300;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StoragePurpose {
    Files,
    Storage,
    User,
    Temporary,
}

impl StoragePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Storage => "storage",
            Self::User => "user",
            Self::Temporary => "temporary",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstanceStorageLocation {
    /// A fixed s3://, az:// or gs:// bucket and purpose-specific project prefix.
    pub uri: String,
    pub prefix: String,
    /// Reference into the lease credential map.
    pub credential_id: String,
    /// Routing options only. Authentication is carried separately.
    pub options: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstanceStorageCredential {
    AwsSession {
        access_key_id: String,
        secret_access_key: String,
        session_token: String,
    },
    AzureSas {
        sas_token: String,
    },
    GcpBearer {
        access_token: String,
    },
}

impl std::fmt::Debug for InstanceStorageCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InstanceStorageCredential([redacted])")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceStorageLease {
    pub instance_id: String,
    pub device_id: String,
    pub device_auth_epoch: u64,
    pub key_epoch: u64,
    pub grant_id: String,
    pub authz_version: u64,
    pub project_id: String,
    pub placement_id: String,
    pub deployment_id: String,
    pub delegating_user_id: String,
    pub access: OnlineProjectAccess,
    /// Effective provider-enforced expiry, including any stricter policy time fence.
    pub expires_at: i64,
    /// Resource grant and device deployment consent deadline, independent of the instance lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_expires_at: Option<i64>,
    pub locations: BTreeMap<StoragePurpose, InstanceStorageLocation>,
    /// Most providers share one credential; Azure signs each directory separately.
    pub credentials: BTreeMap<String, InstanceStorageCredential>,
}
