use super::RuntimeCredentialsTrait;
#[cfg(feature = "aws")]
use crate::credentials::CredentialsAccess;
use crate::state::{AppState, State};
#[cfg(feature = "aws")]
use flow_like::credentials::{
    BucketConfig, SharedCredentials, aws_credentials::AwsSharedCredentials,
};
use flow_like::{
    flow_like_storage::lancedb::{connect, connection::ConnectBuilder},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use serde_json::{json, to_string};
use std::sync::Arc;

#[cfg(feature = "aws")]
#[derive(Clone, Serialize, Deserialize)]
pub struct AwsRuntimeCredentials {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub meta_bucket: String,
    pub content_bucket: String,
    pub logs_bucket: String,
    pub region: String,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub content_path_prefix: Option<String>,
    pub user_content_path_prefix: Option<String>,
}

#[cfg(feature = "aws")]
impl std::fmt::Debug for AwsRuntimeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsRuntimeCredentials")
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("meta_bucket", &self.meta_bucket)
            .field("content_bucket", &self.content_bucket)
            .field("logs_bucket", &self.logs_bucket)
            .field("region", &self.region)
            .field("expiration", &self.expiration)
            .finish()
    }
}

#[cfg(feature = "aws")]
impl From<aws_sdk_sts::types::Credentials> for AwsRuntimeCredentials {
    fn from(credentials: aws_sdk_sts::types::Credentials) -> Self {
        AwsRuntimeCredentials {
            access_key_id: Some(credentials.access_key_id),
            secret_access_key: Some(credentials.secret_access_key),
            session_token: Some(credentials.session_token),
            meta_bucket: std::env::var("META_BUCKET")
                .or_else(|_| std::env::var("META_BUCKET_NAME"))
                .unwrap_or_default(),
            content_bucket: std::env::var("CONTENT_BUCKET")
                .or_else(|_| std::env::var("CONTENT_BUCKET_NAME"))
                .unwrap_or_default(),
            logs_bucket: std::env::var("LOG_BUCKET").unwrap_or_default(),
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }
}

#[cfg(feature = "aws")]
impl AwsRuntimeCredentials {
    pub fn new(meta_bucket: &str, content_bucket: &str, logs_bucket: &str, region: &str) -> Self {
        AwsRuntimeCredentials {
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            meta_bucket: meta_bucket.to_string(),
            content_bucket: content_bucket.to_string(),
            logs_bucket: logs_bucket.to_string(),
            region: region.to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub fn from_env() -> Self {
        let logs_bucket = std::env::var("LOG_BUCKET").unwrap_or_default();
        if logs_bucket.is_empty() {
            tracing::warn!(
                "LOG_BUCKET environment variable is not set - logs will not be persisted"
            );
        }
        AwsRuntimeCredentials {
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok(),
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
            session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
            meta_bucket: std::env::var("META_BUCKET")
                .or_else(|_| std::env::var("META_BUCKET_NAME"))
                .unwrap_or_default(),
            content_bucket: std::env::var("CONTENT_BUCKET")
                .or_else(|_| std::env::var("CONTENT_BUCKET_NAME"))
                .unwrap_or_default(),
            logs_bucket,
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        AwsRuntimeCredentials {
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok(),
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
            session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            region: self.region.clone(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[tracing::instrument(
        name = "AwsRuntimeCredentials::scoped_credentials",
        skip(self, sub, state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(flow_like_types::anyhow!("Sub or App ID cannot be empty"));
        }

        // Validate sub and app_id to prevent path traversal and policy injection
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let role = runtime_role_arn().ok_or_else(|| {
            flow_like_types::anyhow!("RUNTIME_ROLE_ARN environment variable not set")
        })?;

        let client = aws_sdk_sts::Client::new(&state.aws_client);

        let apps_prefix = format!("apps/{}", app_id);
        let db_prefix = format!("{}/storage/db", apps_prefix);
        let user_prefix = format!("users/{}/apps/{}", sub, app_id);
        let runs_prefix = format!("runs/{}", app_id);
        // The writers (`/tmp` presign, HTTP-sink offload) run both segments
        // through `storage_path_segment`, so the prefixes this credential
        // authorises have to as well — a raw `sink:abc` here would name a
        // directory nothing ever writes to.
        let (temporary_user_prefix, temporary_global_prefix) =
            crate::credentials::temporary_prefixes(sub, app_id);
        let (content_path_prefix, user_content_path_prefix) =
            scoped_content_path_prefixes(&apps_prefix, &user_prefix, &mode);

        // Sanitize role_session_name: AWS requires [\w+=,.@-]{2,64}
        let raw_session = format!("{}-{}", sub, app_id);
        let session_name: String = raw_session
            .chars()
            .filter(|c| {
                c.is_alphanumeric()
                    || *c == '-'
                    || *c == '_'
                    || *c == '.'
                    || *c == '@'
                    || *c == '+'
                    || *c == '='
                    || *c == ','
            })
            .take(64)
            .collect();
        let session_name = if session_name.len() < 2 {
            format!("session-{}", &flow_like_types::create_id()[..8])
        } else {
            session_name
        };

        let meta_express = meta_bucket_express_zone();
        let kms_actions = kms_session_actions(&mode, meta_express);
        let policy = match mode {
            CredentialsAccess::EditApp => edit_app_policy(self, &apps_prefix),
            CredentialsAccess::ReadApp => read_app_policy(self, &apps_prefix),
            CredentialsAccess::ReadAppContent => read_app_content_policy(self, &apps_prefix),
            CredentialsAccess::EditAppContent => edit_app_content_policy(self, &apps_prefix),
            CredentialsAccess::ReadAppDb => read_app_content_policy(self, &db_prefix),
            CredentialsAccess::EditAppDb => edit_app_content_policy(self, &db_prefix),
            CredentialsAccess::EditUser => edit_user_policy(self, &user_prefix),
            CredentialsAccess::ReadUser => read_user_policy(self, &user_prefix),
            CredentialsAccess::InvokeNone => {
                invoke_none_policy(self, &user_prefix, &temporary_user_prefix)
            }
            CredentialsAccess::InvokeRead => invoke_read_policy(
                self,
                &apps_prefix,
                &user_prefix,
                &temporary_user_prefix,
                &temporary_global_prefix,
            ),
            CredentialsAccess::InvokeWrite => invoke_read_write_policy(
                self,
                &apps_prefix,
                &user_prefix,
                &temporary_user_prefix,
                &temporary_global_prefix,
            ),
            CredentialsAccess::ServerExecute => server_execute_policy(
                self,
                &apps_prefix,
                &user_prefix,
                &runs_prefix,
                &temporary_user_prefix,
                &temporary_global_prefix,
            ),
            CredentialsAccess::ShadowExecute => shadow_execute_policy(
                self,
                &apps_prefix,
                &user_prefix,
                &runs_prefix,
                &temporary_user_prefix,
                &temporary_global_prefix,
            ),
            CredentialsAccess::ReadLogs => read_logs_policy(self, &runs_prefix),
        };

        // A bucket encrypted with a customer-managed key answers every S3 call
        // with AccessDenied unless the session may also use the key.
        let policy = with_kms_session_permissions(policy, &mode, kms_actions, &self.region);

        let policy = to_string(&policy)
            .map_err(|e| flow_like_types::anyhow!("Failed to serialize policy: {}", e))?;

        let credentials = client
            .assume_role()
            .role_arn(role)
            .role_session_name(session_name)
            .policy(policy)
            .duration_seconds(3600) // 1 hour
            .send()
            .await?;

        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            access_key_id: credentials
                .credentials()
                .map(|c| c.access_key_id().to_string()),
            secret_access_key: credentials
                .credentials()
                .map(|c| c.secret_access_key().to_string()),
            session_token: credentials
                .credentials()
                .map(|c| c.session_token().to_string()),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            region: self.region.clone(),
            expiration: Some(chrono_expiration),
            content_path_prefix,
            user_content_path_prefix,
        })
    }
}

#[cfg(feature = "aws")]
fn scoped_content_path_prefixes(
    apps_prefix: &str,
    user_prefix: &str,
    mode: &CredentialsAccess,
) -> (Option<String>, Option<String>) {
    let app = matches!(
        mode,
        CredentialsAccess::EditApp
            | CredentialsAccess::ReadApp
            | CredentialsAccess::ReadAppContent
            | CredentialsAccess::EditAppContent
            | CredentialsAccess::ReadAppDb
            | CredentialsAccess::EditAppDb
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::ShadowExecute
    )
    .then(|| apps_prefix.to_string());

    let user = matches!(
        mode,
        CredentialsAccess::EditUser
            | CredentialsAccess::ReadUser
            | CredentialsAccess::InvokeNone
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::ShadowExecute
    )
    .then(|| user_prefix.to_string());

    (app, user)
}

/// STS caps a session policy at 2048 characters of plaintext.
const STS_POLICY_MAX_CHARS: usize = 2048;

/// The role every scoped credential is minted from. Deployment-constant, and
/// `scoped_credentials` runs on every handout, so it is read once.
#[cfg(feature = "aws")]
fn runtime_role_arn() -> Option<&'static str> {
    static RUNTIME_ROLE_ARN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    RUNTIME_ROLE_ARN
        .get_or_init(|| non_empty_env("RUNTIME_ROLE_ARN"))
        .as_deref()
}

#[cfg(feature = "aws")]
fn env_flag(var: &str) -> bool {
    std::env::var(var)
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

#[cfg(feature = "aws")]
fn non_empty_env(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Endpoint, S3 Express and SSE-KMS settings for the three buckets.
///
/// All of it comes from the environment and none of it changes within a
/// process, but `into_shared_credentials` runs on every credential handout —
/// so the whole block is resolved once and cloned from there.
#[cfg(feature = "aws")]
struct BucketConfigs {
    meta: Option<BucketConfig>,
    content: Option<BucketConfig>,
    logs: Option<BucketConfig>,
}

#[cfg(feature = "aws")]
fn bucket_configs() -> &'static BucketConfigs {
    static BUCKET_CONFIGS: std::sync::OnceLock<BucketConfigs> = std::sync::OnceLock::new();
    BUCKET_CONFIGS.get_or_init(|| {
        // Only the *explicitly configured* KMS keys become write headers.
        // A discovered key is one the bucket already applies by itself, so
        // repeating it on every request would buy nothing; naming it here is
        // how a deployment satisfies a bucket policy that refuses writes
        // arriving without `x-amz-server-side-encryption`.
        let kms = KmsKeys::configured();
        let kms_bucket_key = env_flag("S3_KMS_BUCKET_KEY");

        // A bucket with no endpoint, no express flag and no key needs no
        // config at all.
        let config = |endpoint: Option<String>, express: bool, kms_key_arn: Option<String>| {
            (endpoint.is_some() || express || kms_key_arn.is_some()).then_some(BucketConfig {
                endpoint,
                express,
                kms_key_arn,
                kms_bucket_key,
            })
        };

        BucketConfigs {
            meta: config(
                non_empty_env("META_BUCKET_ENDPOINT"),
                env_flag("META_BUCKET_EXPRESS_ZONE"),
                kms.meta.clone(),
            ),
            content: config(
                non_empty_env("CONTENT_BUCKET_ENDPOINT"),
                env_flag("CONTENT_BUCKET_EXPRESS_ZONE"),
                kms.content.clone(),
            ),
            logs: config(
                non_empty_env("LOGS_BUCKET_ENDPOINT"),
                env_flag("LOGS_BUCKET_EXPRESS_ZONE"),
                kms.logs.clone(),
            ),
        }
    })
}

/// Whether the meta bucket is an S3 Express directory bucket. On a directory
/// bucket the CreateSession token — not per-object IAM — authorizes every
/// zonal request, so the two bucket flavors need disjoint policy statements.
/// Emitting both flavors in one session policy pushed AssumeRole past its
/// packed-policy budget (PackedPolicyTooLargeException at dispatch).
#[cfg(feature = "aws")]
fn meta_bucket_express_zone() -> bool {
    bucket_configs()
        .meta
        .as_ref()
        .is_some_and(|config| config.express)
}

/// Whether any of the three buckets is an S3 Express directory bucket.
///
/// Only relevant to the KMS grant: directory buckets reach KMS as
/// `s3express`, so that service principal has to be allowed alongside `s3` —
/// but only where such a bucket exists, because every entry costs policy
/// budget.
#[cfg(feature = "aws")]
fn any_express_zone() -> bool {
    let configs = bucket_configs();
    [&configs.meta, &configs.content, &configs.logs]
        .into_iter()
        .any(|config| config.as_ref().is_some_and(|config| config.express))
}

// ============================================================================
// SSE-KMS (customer-managed keys)
// ============================================================================

/// The customer-managed KMS keys guarding the three buckets.
///
/// SSE-S3 and the AWS-managed `aws/s3` key need nothing here — S3 authorizes
/// those itself. A **customer** managed key does not: every `GetObject`
/// additionally needs `kms:Decrypt` on the key and every `PutObject`
/// additionally needs `kms:GenerateDataKey`. Session policies *intersect* with
/// the runtime role, so a policy naming only `s3:*` actions denies every
/// request against a CMK-encrypted bucket regardless of what the role itself
/// may do — which is why these grants have to be minted alongside the S3 ones
/// rather than left to the role.
#[cfg(feature = "aws")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KmsKeys {
    pub meta: Option<String>,
    pub content: Option<String>,
    pub logs: Option<String>,
}

#[cfg(feature = "aws")]
impl KmsKeys {
    /// The explicitly configured keys, resolved once. `S3_KMS_KEY_ARN` covers
    /// the common case of one key for every bucket; the per-bucket variables
    /// override it.
    pub fn configured() -> &'static Self {
        static CONFIGURED: std::sync::OnceLock<KmsKeys> = std::sync::OnceLock::new();
        CONFIGURED.get_or_init(|| {
            let shared = non_empty_env("S3_KMS_KEY_ARN");
            let for_bucket = |var: &str| non_empty_env(var).or_else(|| shared.clone());

            KmsKeys {
                meta: for_bucket("META_BUCKET_KMS_KEY_ARN"),
                content: for_bucket("CONTENT_BUCKET_KMS_KEY_ARN"),
                logs: for_bucket("LOG_BUCKET_KMS_KEY_ARN"),
            }
        })
    }
}

/// Which buckets a credential mode touches, and therefore whose keys a pinned
/// KMS statement may name. It has to mirror the mode's S3 statements exactly:
/// a key named for a bucket the session cannot reach is a grant that buys
/// nothing, and one omitted for a bucket it can reach denies every request.
#[cfg(feature = "aws")]
struct KmsScope {
    meta: bool,
    content: bool,
    logs: bool,
}

#[cfg(feature = "aws")]
impl KmsScope {
    fn of(mode: &CredentialsAccess) -> Self {
        Self {
            // The executor modes never open the meta bucket: boards arrive as
            // presigned artifacts, widgets via the hub.
            meta: matches!(
                mode,
                CredentialsAccess::EditApp | CredentialsAccess::ReadApp
            ),
            // Every mode but `ReadLogs` works on app or user content.
            content: !matches!(mode, CredentialsAccess::ReadLogs),
            logs: matches!(
                mode,
                CredentialsAccess::ServerExecute
                    | CredentialsAccess::ShadowExecute
                    | CredentialsAccess::ReadLogs
            ),
        }
    }
}

/// The KMS actions a mode needs, tracking its effective S3 access: a reader
/// only unwraps an existing data key, a writer also has S3 wrap a new one.
#[cfg(feature = "aws")]
fn kms_session_actions(mode: &CredentialsAccess, meta_express: bool) -> &'static [&'static str] {
    const READ: &[&str] = &["kms:Decrypt"];
    const READ_WRITE: &[&str] = &["kms:Decrypt", "kms:GenerateDataKey"];

    let writes_objects = matches!(
        mode,
        CredentialsAccess::EditApp
            | CredentialsAccess::EditAppContent
            | CredentialsAccess::EditAppDb
            | CredentialsAccess::EditUser
            // Despite their names, all invoke modes may write user/tmp data.
            | CredentialsAccess::InvokeNone
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            // A shadow run may not touch app or user content, but it still
            // writes scratch and appends run logs.
            | CredentialsAccess::ShadowExecute
    );

    // S3 Express requires both permissions when it creates an SSE-KMS-backed
    // session, including a session constrained to ReadOnly.
    if writes_objects || (meta_express && matches!(mode, CredentialsAccess::ReadApp)) {
        READ_WRITE
    } else {
        READ
    }
}

/// The configured keys for the buckets a mode touches, or `None` when none of
/// them is pinned.
///
/// A bucket left unpinned contributes nothing: either it is not on a
/// customer-managed key and needs no KMS grant at all, or the deployment is
/// relying on the role policy for it. `S3_KMS_KEY_ARN` pins all three at once
/// and is the right lever whenever more than one bucket shares a key.
#[cfg(feature = "aws")]
fn scoped_kms_resources(
    keys: &'static KmsKeys,
    mode: &CredentialsAccess,
) -> Option<Vec<&'static str>> {
    let scope = KmsScope::of(mode);
    let mut resources: Vec<&'static str> = Vec::with_capacity(3);
    let candidates = [
        scope.meta.then_some(keys.meta.as_deref()).flatten(),
        scope.content.then_some(keys.content.as_deref()).flatten(),
        scope.logs.then_some(keys.logs.as_deref()).flatten(),
    ];
    for key in candidates.into_iter().flatten() {
        if !resources.contains(&key) {
            resources.push(key);
        }
    }

    (!resources.is_empty()).then_some(resources)
}

/// The KMS statement for a session policy.
///
/// `Resource: "*"` is the default and is not a widening: STS intersects the
/// session policy with the assumed role, whose bucket-module policies already
/// pin the exact CMK, `kms:ViaService` and the S3 encryption context. Naming
/// the keys here as well would only spend the 2048-character policy budget to
/// restate what the role already enforces.
///
/// A deployment that would rather not lean on the role policy alone pins the
/// keys with `S3_KMS_KEY_ARN` (or the per-bucket variables). The statement then
/// names those ARNs and carries its own `kms:ViaService` fence, so a leaked
/// session cannot reach KMS except through S3.
#[cfg(feature = "aws")]
fn kms_statement(
    keys: &'static KmsKeys,
    mode: &CredentialsAccess,
    actions: &[&str],
    region: &str,
    express: bool,
) -> flow_like_types::Value {
    let mut statement = json!({
        "Effect": "Allow",
        "Action": actions,
        "Resource": "*"
    });

    let Some(resources) = scoped_kms_resources(keys, mode) else {
        return statement;
    };

    // Directory buckets reach KMS as `s3express`, so that service principal
    // joins the fence only where such a bucket exists.
    let mut via_service = vec![format!("s3.{}.amazonaws.com", region)];
    if express {
        via_service.push(format!("s3express.{}.amazonaws.com", region));
    }

    statement["Resource"] = json!(resources);
    statement["Condition"] = json!({
        "StringEquals": { "kms:ViaService": via_service }
    });
    statement
}

/// KMS permissions have to appear in the inline AssumeRole policy as well as on
/// the assumed role. STS intersects the two, so omitting KMS here silently
/// strips the role's KMS grant from every scoped credential — including the
/// credentials behind presigned URLs, which is where it surfaces as an opaque
/// AccessDenied long after the mint succeeded.
#[cfg(feature = "aws")]
fn with_kms_session_permissions(
    mut policy: flow_like_types::Value,
    mode: &CredentialsAccess,
    actions: &[&str],
    region: &str,
) -> flow_like_types::Value {
    let statement = kms_statement(
        KmsKeys::configured(),
        mode,
        actions,
        region,
        any_express_zone(),
    );

    policy["Statement"]
        .as_array_mut()
        .expect("AWS scoped policy builders must emit a Statement array")
        .push(statement);
    policy
}

#[cfg(feature = "aws")]
impl AwsRuntimeCredentials {
    /// The concrete AWS view of these credentials, including the per-bucket
    /// endpoint / express / SSE-KMS configuration read from the environment.
    fn aws_shared_credentials(&self) -> AwsSharedCredentials {
        let configs = bucket_configs();

        AwsSharedCredentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            meta_config: configs.meta.clone(),
            content_config: configs.content.clone(),
            logs_config: configs.logs.clone(),
            region: self.region.clone(),
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
        }
    }
}

#[cfg(feature = "aws")]
#[async_trait]
impl RuntimeCredentialsTrait for AwsRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        SharedCredentials::Aws(self.aws_shared_credentials())
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials().to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials()
            .to_db_scoped(sub, app_id)
            .await
    }

    #[tracing::instrument(
        name = "AwsRuntimeCredentials::to_state",
        skip(self, state),
        level = "debug"
    )]
    async fn to_state(&self, state: AppState) -> Result<FlowLikeState> {
        let (meta_store, content_store) = {
            use flow_like_types::tokio;

            tokio::join!(
                async { self.into_shared_credentials().to_store(true).await },
                async { self.into_shared_credentials().to_store(false).await },
            )
        };
        let http_client = HTTPClient::new_without_refetch();

        let meta_store = meta_store?;
        let content_store = content_store?;

        let mut config = {
            let mut cfg = FlowLikeConfig::with_default_store(content_store);
            cfg.register_app_meta_store(meta_store.clone());
            cfg
        };

        let (content_bucket, logs_bucket, key, secret, token) = (
            self.content_bucket.clone(),
            self.logs_bucket.clone(),
            self.access_key_id
                .clone()
                .ok_or(anyhow!("AWS_ACCESS_KEY_ID is not set"))?,
            self.secret_access_key
                .clone()
                .ok_or(anyhow!("AWS_SECRET_ACCESS_KEY is not set"))?,
            self.session_token
                .clone()
                .ok_or(anyhow!("SESSION_TOKEN is not set"))?,
        );

        // Run logs live on the logs bucket and the project/user databases on
        // the content bucket, so each takes its own SSE-KMS configuration.
        let shared = self.aws_shared_credentials();
        let logs_sse = flow_like::credentials::aws_credentials::sse_kms_storage_options(
            shared.logs_config.as_ref(),
        );
        let content_sse = flow_like::credentials::aws_credentials::sse_kms_storage_options(
            shared.content_config.as_ref(),
        );

        config.register_build_logs_database(Arc::new(make_s3_builder(
            logs_bucket,
            key.clone(),
            secret.clone(),
            token.clone(),
            logs_sse,
        )));
        config.register_build_project_database(Arc::new(make_s3_builder(
            content_bucket.clone(),
            key.clone(),
            secret.clone(),
            token.clone(),
            content_sse.clone(),
        )));
        config.register_build_user_database(Arc::new(make_s3_builder(
            content_bucket,
            key,
            secret,
            token,
            content_sse,
        )));

        let mut flow_like_state = FlowLikeState::new(config, http_client);

        flow_like_state.model_provider_config = state.provider.clone();
        flow_like_state.node_registry.write().await.node_registry = state.registry.clone();

        Ok(flow_like_state)
    }
}

fn make_s3_builder(
    bucket: String,
    access_key: String,
    secret_key: String,
    session_token: String,
    sse_options: Vec<(String, String)>,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("s3://{}/{}", bucket, path);
        let mut builder = connect(&url)
            .storage_option("aws_access_key_id".to_string(), access_key.clone())
            .storage_option("aws_secret_access_key".to_string(), secret_key.clone())
            .storage_option("aws_session_token".to_string(), session_token.clone());
        for (key, value) in &sse_options {
            builder = builder.storage_option(key.clone(), value.clone());
        }
        builder
    }
}

/// Run logs (`runs/{app_id}`) are written only by the server executor
/// (`ServerExecute`) and read only through the API (`ReadLogs`); the desktop
/// keeps its own local log store. None of the client-facing invoke policies
/// therefore names that prefix.
fn invoke_read_write_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
    user_prefix: &str,
    temporary_user_prefix: &str,
    temporary_global_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                        format!("{}/*", user_prefix),
                        format!("{}/*", temporary_user_prefix),
                        format!("{}/*", temporary_global_prefix)
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_user_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_global_prefix),
            ],
          }
        ],
    });

    policy
}

/// The executor's own grant: app and user content, scratch, and append-only
/// run logs. Nothing on the meta bucket — the board reaches the executor as a
/// presigned compiled artifact and widgets come from the hub, so this
/// credential never needs to address meta at all. On a directory bucket that
/// distinction is the whole game: `CreateSession` cannot be scoped below the
/// bucket, so the only meta grant that is not bucket-wide is none.
fn server_execute_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
    user_prefix: &str,
    runs_prefix: &str,
    temporary_user_prefix: &str,
    temporary_global_prefix: &str,
) -> flow_like_types::Value {
    let statements = vec![
        json!({
          "Effect": "Allow",
          "Action": [
              "s3:ListBucket"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}", credentials.content_bucket)
          ],
          "Condition": {
              "StringLike": {
                  "s3:prefix": [
                      format!("{}/*", apps_prefix),
                      format!("{}/*", user_prefix),
                      format!("{}/*", temporary_user_prefix),
                      format!("{}/*", temporary_global_prefix)
                  ]
              }
          }
        }),
        json!({
          "Effect": "Allow",
          "Action": [
              "s3:GetObject",
              "s3:PutObject",
              "s3:DeleteObject"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_user_prefix),
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_global_prefix),
          ],
        }),
        json!({
          "Effect": "Allow",
          "Action": [
              "s3:ListBucket"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}", credentials.logs_bucket)
          ],
          "Condition": {
              "StringLike": {
                  "s3:prefix": [
                      format!("{}/*", runs_prefix)
                  ]
              }
          }
        }),
        // Run logs are append-only: the executor creates and appends Lance
        // tables (fresh data files, `.txn` files, conditional-put manifests)
        // and never deletes; Lance's auto-cleanup is best-effort and logs on
        // denial. Without DeleteObject a compromised run cannot erase or
        // rewrite another run's audit trail.
        json!({
          "Effect": "Allow",
          "Action": [
              "s3:GetObject",
              "s3:PutObject"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}/{}/*", credentials.logs_bucket, runs_prefix),
          ],
        }),
    ];
    json!({
        "Version": "2012-10-17",
        "Statement": statements,
    })
}

/// `server_execute_policy` for a shadow/replay run: identical list, log and
/// meta statements, but `s3:PutObject`/`s3:DeleteObject` are dropped on the
/// app and user content prefixes — a shadow run reads live content and may
/// never mutate it. Scratch (`tmp/*`) keeps read/write so request-file
/// offloads and cache paths still work, and run logs stay append-only so the
/// shadow run itself is recorded.
fn shadow_execute_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
    user_prefix: &str,
    runs_prefix: &str,
    temporary_user_prefix: &str,
    temporary_global_prefix: &str,
) -> flow_like_types::Value {
    let mut policy = server_execute_policy(
        credentials,
        apps_prefix,
        user_prefix,
        runs_prefix,
        temporary_user_prefix,
        temporary_global_prefix,
    );

    // Statement 1 is the content-bucket object statement (see
    // `server_execute_policy`); split it into read-only app/user content plus
    // read-write scratch. Rebuilding it here instead of duplicating the whole
    // policy keeps the two modes from drifting, and the split stays within the
    // STS packed-policy budget the ServerExecute tests pin.
    policy["Statement"][1] = json!({
      "Effect": "Allow",
      "Action": [
          "s3:GetObject"
      ],
      "Resource": [
          format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
          format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
      ],
    });
    policy["Statement"]
        .as_array_mut()
        .expect("server_execute_policy statements are an array")
        .push(json!({
          "Effect": "Allow",
          "Action": [
              "s3:GetObject",
              "s3:PutObject",
              "s3:DeleteObject"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_user_prefix),
              format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_global_prefix),
          ],
        }));

    policy
}

fn invoke_read_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
    user_prefix: &str,
    temporary_user_prefix: &str,
    temporary_global_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                        format!("{}/*", user_prefix),
                        format!("{}/*", temporary_user_prefix),
                        format!("{}/*", temporary_global_prefix)
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_global_prefix),
            ],
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_user_prefix),
            ],
          }
        ],
    });

    policy
}

fn invoke_none_policy(
    credentials: &AwsRuntimeCredentials,
    user_prefix: &str,
    temporary_user_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket),
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", user_prefix),
                        format!("{}/*", temporary_user_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, temporary_user_prefix),
            ],
          }
        ],
    });

    policy
}

fn edit_app_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.meta_bucket),
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
                format!("arn:aws:s3express:::{}/{}/*", credentials.meta_bucket, apps_prefix),
            ],
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3express:CreateSession",
            ],
            "Resource": [
                "*"
            ]
          }
        ],
    });

    policy
}

/// Write scope restricted to the **content bucket** only — used
/// wherever scoped credentials cross the trust boundary to a client
/// (presign-data-access uploads, fork-online-begin bundle uploads).
/// Boards / events / widgets / templates / pages on the meta bucket
/// must always be written through the API so that role-permission
/// gates (`WriteBoards`, `WriteEvents`, `WriteTemplates`, …) and
/// per-resource validation (event-schedule checks, page-event
/// coupling, sink registration, secret stripping on write) run on
/// every change. The full-fat `EditApp` policy includes meta-bucket
/// `s3:Put/Get/Delete`, which would let a misbehaving client drop
/// arbitrary `.board` / `.event` files server-side and bypass those
/// guards.
fn edit_app_content_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
            ],
          }
        ],
    });

    policy
}

/// Read scope restricted to the **content bucket** only — used by the
/// fork-an-app flow. Boards / events / widgets / templates / pages
/// live in the meta bucket and may contain secrets that the server
/// strips before they leave through the API; granting the holder of
/// these credentials read access to the meta bucket would let them
/// bypass that sanitization. Hence: list + get on the content bucket
/// only, no meta-bucket statements at all.
fn read_app_content_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
            ],
          }
        ],
    });

    policy
}

fn read_app_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.meta_bucket),
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", apps_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, apps_prefix),
                format!("arn:aws:s3express:::{}/{}/*", credentials.meta_bucket, apps_prefix),
            ],
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3express:CreateSession",
            ],
            "Resource": [
                "*"
            ],
            // On a directory bucket the session token — not per-object IAM —
            // authorizes every zonal request, and object_store asks for the
            // maximum privilege. Pin the session to ReadOnly so this mode
            // cannot write to the meta bucket.
            "Condition": {
                "StringEquals": {
                    "s3express:SessionMode": "ReadOnly"
                }
            }
          }
        ],
    });

    policy
}

fn edit_user_policy(
    credentials: &AwsRuntimeCredentials,
    user_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", user_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
                "s3:PutObject",
                "s3:DeleteObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
            ],
          }
        ],
    });

    policy
}

fn read_user_policy(
    credentials: &AwsRuntimeCredentials,
    user_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.content_bucket)
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", user_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, user_prefix),
            ],
          }
        ],
    });

    policy
}

/// `ServerExecute` writes run logs to the **logs** bucket, so that is where
/// `ReadLogs` has to look. Naming the content bucket here pointed the reader
/// at a prefix nothing writes.
fn read_logs_policy(
    credentials: &AwsRuntimeCredentials,
    runs_prefix: &str,
) -> flow_like_types::Value {
    let policy = json!({
        "Version": "2012-10-17",
        "Statement": [
          {
            "Effect": "Allow",
            "Action": [
                "s3:ListBucket"
            ],
            "Resource": [
                format!("arn:aws:s3:::{}", credentials.logs_bucket),
            ],
            "Condition": {
                "StringLike": {
                    "s3:prefix": [
                        format!("{}/*", runs_prefix),
                    ]
                }
            }
          },
          {
            "Effect": "Allow",
            "Action": [
                "s3:GetObject",
            ],
            "Resource": [
                format!("arn:aws:s3:::{}/{}/*", credentials.logs_bucket, runs_prefix),
            ],
          }
        ],
    });

    policy
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(test, feature = "aws"))]
mod tests {
    use super::*;
    use crate::credentials::RuntimeCredentialsTrait;
    use flow_like_storage::Path;
    use flow_like_storage::object_store::ObjectStore;
    use flow_like_types::json::{from_str, to_string};
    use flow_like_types::tokio;

    #[tokio::test]
    #[ignore]
    async fn test_aws_master_credentials_setup() {
        let creds = AwsRuntimeCredentials::from_env();
        assert!(
            creds.access_key_id.is_some(),
            "AWS_ACCESS_KEY_ID must be set"
        );
        assert!(
            creds.secret_access_key.is_some(),
            "AWS_SECRET_ACCESS_KEY must be set"
        );
        assert!(
            !creds.meta_bucket.is_empty(),
            "META_BUCKET or META_BUCKET_NAME must be set"
        );
        assert!(
            !creds.content_bucket.is_empty(),
            "CONTENT_BUCKET or CONTENT_BUCKET_NAME must be set"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_aws_master_credentials_can_write() {
        let creds = AwsRuntimeCredentials::from_env();
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!(
            "test/master-write-test-{}.txt",
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::AWS(s) => {
                s.put(&path, b"test content".to_vec().into())
                    .await
                    .expect("Master credentials should be able to write");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected AWS store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_aws_master_credentials_can_read() {
        let creds = AwsRuntimeCredentials::from_env();
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!("test/master-read-test-{}.txt", flow_like_types::create_id());
        let path = Path::from(test_path.as_str());
        let content = b"read test content";

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::AWS(s) => {
                s.put(&path, content.to_vec().into())
                    .await
                    .expect("Setup: write should succeed");

                let result = s.get(&path).await.expect("Read should succeed");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), content);

                s.delete(&path).await.ok();
            }
            _ => panic!("Expected AWS store"),
        }
    }

    fn test_aws_runtime_credentials() -> AwsRuntimeCredentials {
        AwsRuntimeCredentials {
            access_key_id: Some("AKIATEST".to_string()),
            secret_access_key: Some("secret".to_string()),
            session_token: Some("token".to_string()),
            meta_bucket: "meta-secret".to_string(),
            content_bucket: "content-data".to_string(),
            logs_bucket: "logs".to_string(),
            region: "us-east-1".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    /// The regression this file carried: the policies interpolated the raw
    /// `sub` while every writer ran it through `storage_path_segment`, so a
    /// `sink:…` subject — which is what `trigger_http` uses whenever no PAT
    /// user resolves — produced a policy for a directory nothing writes to.
    #[test]
    fn test_aws_temporary_prefixes_match_what_the_writers_build() {
        let (temporary_user_prefix, _) =
            crate::credentials::temporary_prefixes("sink:abc", "app-1");
        let policy = to_string(&invoke_none_policy(
            &test_aws_runtime_credentials(),
            "users/sink:abc/apps/app-1",
            &temporary_user_prefix,
        ))
        .expect("policy should serialize");

        assert!(
            policy.contains(&format!("content-data/{temporary_user_prefix}/*")),
            "policy must name the prefix the writers use: {policy}"
        );
        assert!(
            !policy.contains("tmp/user/sink:abc"),
            "the raw subject must never reach a scratch prefix: {policy}"
        );
    }

    #[test]
    fn test_aws_invoke_policies_do_not_grant_meta_access() {
        let creds = test_aws_runtime_credentials();
        let apps_prefix = "apps/app-1";
        let user_prefix = "users/user-1/apps/app-1";
        let runs_prefix = "runs/app-1";
        let temporary_user_prefix = "tmp/user/user-1/apps/app-1";
        let temporary_global_prefix = "tmp/global/apps/app-1";

        let policies = [
            (
                "InvokeNone",
                invoke_none_policy(&creds, user_prefix, temporary_user_prefix),
            ),
            (
                "InvokeRead",
                invoke_read_policy(
                    &creds,
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ),
            ),
            (
                "InvokeWrite",
                invoke_read_write_policy(
                    &creds,
                    apps_prefix,
                    user_prefix,
                    temporary_user_prefix,
                    temporary_global_prefix,
                ),
            ),
        ];

        for (mode, policy) in policies {
            let policy = to_string(&policy).expect("policy should serialize");
            assert!(
                !policy.contains("meta-secret"),
                "{mode} must not grant access to the meta bucket"
            );
            assert!(
                !policy.contains("s3express:CreateSession"),
                "{mode} should not need S3 Express sessions without meta access"
            );
            assert!(
                !policy.contains(runs_prefix),
                "{mode} must not reach the app's run logs; only ServerExecute writes and ReadLogs reads them"
            );
        }
    }

    #[test]
    fn test_aws_invoke_none_does_not_grant_app_content_access() {
        let creds = test_aws_runtime_credentials();
        let policy = invoke_none_policy(
            &creds,
            "users/user-1/apps/app-1",
            "tmp/user/user-1/apps/app-1",
        );
        let policy = to_string(&policy).expect("policy should serialize");

        assert!(
            !policy.contains("content-data/apps/app-1/*"),
            "InvokeNone must not grant app content access"
        );
        assert!(policy.contains("content-data/users/user-1/apps/app-1/*"));
        assert!(policy.contains("content-data/tmp/user/user-1/apps/app-1/*"));
        assert!(!policy.contains("content-data/runs/app-1/*"));
    }

    #[test]
    fn test_aws_invoke_none_shared_shape_has_no_app_content_prefix() {
        let (app, user) = scoped_content_path_prefixes(
            "apps/app-1",
            "users/user-1/apps/app-1",
            &CredentialsAccess::InvokeNone,
        );

        assert_eq!(app, None);
        assert_eq!(user, Some("users/user-1/apps/app-1".to_string()));
    }

    #[test]
    /// The executor's board arrives as a presigned compiled artifact and its
    /// widgets come from the hub, so its credential must not be able to
    /// address the meta bucket at all — on a directory bucket any
    /// `CreateSession` grant is bucket-wide, so "none" is the only scope.
    #[test]
    fn test_aws_server_execute_grants_no_meta_access() {
        let creds = test_aws_runtime_credentials();
        let policy = server_execute_policy(
            &creds,
            "apps/app-1",
            "users/user-1/apps/app-1",
            "runs/app-1",
            "tmp/user/user-1/apps/app-1",
            "tmp/global/apps/app-1",
        );
        let statements = policy["Statement"]
            .as_array()
            .expect("policy statements should be an array");
        for statement in statements {
            assert!(
                !statement["Resource"].to_string().contains("meta-secret"),
                "ServerExecute must not name the meta bucket: {statement}"
            );
        }
        let json = to_string(&policy).expect("policy should serialize");
        assert!(
            !json.contains("s3express"),
            "ServerExecute must not be able to open an express session: {json}"
        );
        assert!(json.contains("content-data/apps/app-1/*"));
        assert!(json.contains("logs/runs/app-1/*"));
        let logs_actions = statements
            .iter()
            .find(|statement| statement["Resource"].to_string().contains("logs/runs"))
            .expect("server execute should include run-log object access")["Action"]
            .to_string();
        assert!(logs_actions.contains("PutObject"));
        assert!(
            !logs_actions.contains("DeleteObject"),
            "ServerExecute run logs are append-only"
        );
    }

    /// A shadow run reads live app/user content but may never mutate it;
    /// scratch stays writable and run logs stay append-only.
    #[test]
    fn test_aws_shadow_execute_drops_content_writes_but_keeps_reads() {
        let creds = test_aws_runtime_credentials();
        let apps_prefix = "apps/app-1";
        let user_prefix = "users/user-1/apps/app-1";
        let policy = shadow_execute_policy(
            &creds,
            apps_prefix,
            user_prefix,
            "runs/app-1",
            "tmp/user/user-1/apps/app-1",
            "tmp/global/apps/app-1",
        );
        let statements = policy["Statement"]
            .as_array()
            .expect("policy statements should be an array");

        for statement in statements {
            let actions = statement["Action"].to_string();
            let resources = statement["Resource"].to_string();
            if !actions.contains("PutObject") && !actions.contains("DeleteObject") {
                continue;
            }
            assert!(
                !resources.contains(&format!("content-data/{apps_prefix}/"))
                    && !resources.contains(&format!("content-data/{user_prefix}/")),
                "ShadowExecute must not write app or user content: {statement}"
            );
        }

        let json = to_string(&policy).expect("policy should serialize");
        assert!(
            json.contains("content-data/apps/app-1/*"),
            "app content stays readable: {json}"
        );
        assert!(
            json.contains("content-data/tmp/user/user-1/apps/app-1/*"),
            "scratch stays reachable: {json}"
        );
        let logs_actions = statements
            .iter()
            .find(|statement| statement["Resource"].to_string().contains("logs/runs"))
            .expect("shadow execute still records run logs")["Action"]
            .to_string();
        assert!(logs_actions.contains("PutObject"));
        assert!(!logs_actions.contains("DeleteObject"));
    }

    /// The shadow variant adds one statement to `server_execute_policy`; both
    /// flavors must keep headroom under the 2048-char STS plaintext cap with
    /// realistic id lengths.
    #[test]
    fn test_aws_shadow_execute_stays_within_policy_budget() {
        let creds = test_aws_runtime_credentials();
        let app = "apps/v8f5p73w00itor03zlrai22w";
        let user = "users/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let runs = "runs/v8f5p73w00itor03zlrai22w";
        let tmp_user = "tmp/user/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let tmp_global = "tmp/global/apps/v8f5p73w00itor03zlrai22w";

        let policy = shadow_execute_policy(&creds, app, user, runs, tmp_user, tmp_global);
        let json = to_string(&policy).expect("policy should serialize");
        assert!(
            json.len() < 2000,
            "ShadowExecute policy grew to {} chars",
            json.len()
        );
    }

    /// Carrying meta statements once pushed AssumeRole past its packed-policy
    /// budget in production (PackedPolicyTooLargeException at dispatch). With
    /// the meta grant gone entirely the policy is smaller than either old
    /// flavor; keep it that way.
    #[test]
    fn test_aws_server_execute_stays_within_policy_budget() {
        let creds = test_aws_runtime_credentials();
        // Realistic id lengths: generated app ids and IdP subjects are long,
        // and every prefix repeats them several times.
        let app = "apps/v8f5p73w00itor03zlrai22w";
        let user = "users/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let runs = "runs/v8f5p73w00itor03zlrai22w";
        let tmp_user = "tmp/user/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let tmp_global = "tmp/global/apps/v8f5p73w00itor03zlrai22w";

        let policy = server_execute_policy(&creds, app, user, runs, tmp_user, tmp_global);
        let json = to_string(&policy).expect("policy should serialize");
        assert!(!json.contains("meta-secret"));
        assert!(!json.contains("s3express"));
        // STS caps the plaintext at 2048 characters and packs it into a
        // shared binary quota; keep real headroom.
        assert!(
            json.len() < 1600,
            "ServerExecute policy grew to {} chars",
            json.len()
        );
    }

    /// On a directory (S3 Express) bucket, per-object IAM statements are not
    /// evaluated — the CreateSession mode is the only thing that separates
    /// read from write. Read-only modes must therefore pin the session mode.
    #[test]
    fn test_aws_read_only_meta_modes_pin_readonly_express_session() {
        let creds = test_aws_runtime_credentials();
        let apps_prefix = "apps/app-1";

        let create_session_mode = |policy: flow_like_types::Value| -> Option<String> {
            policy["Statement"]
                .as_array()
                .expect("statements")
                .iter()
                .find(|s| s["Action"].to_string().contains("s3express:CreateSession"))
                .map(|s| s["Condition"]["StringEquals"]["s3express:SessionMode"].to_string())
        };

        assert_eq!(
            create_session_mode(read_app_policy(&creds, apps_prefix)).as_deref(),
            Some("\"ReadOnly\""),
            "ReadApp must only obtain ReadOnly express sessions"
        );
        assert_eq!(
            create_session_mode(server_execute_policy(
                &creds,
                apps_prefix,
                "users/user-1/apps/app-1",
                "runs/app-1",
                "tmp/user/user-1/apps/app-1",
                "tmp/global/apps/app-1",
            )),
            None,
            "ServerExecute must not be able to open any express session"
        );
        assert_eq!(
            create_session_mode(edit_app_policy(&creds, apps_prefix)).as_deref(),
            Some("null"),
            "EditApp legitimately writes the meta bucket"
        );
    }

    /// `KmsKeys::configured()` is a process-wide `OnceLock` over the
    /// environment, so the pinned path is exercised through `kms_statement`
    /// with explicit keys instead of mutating `std::env` under a parallel
    /// test runner. Leaked here so the borrow can be `'static`.
    fn test_kms_keys() -> &'static KmsKeys {
        Box::leak(Box::new(KmsKeys {
            meta: Some(
                "arn:aws:kms:us-east-1:123456789012:key/11111111-1111-1111-1111-111111111111"
                    .to_string(),
            ),
            content: Some(
                "arn:aws:kms:us-east-1:123456789012:key/22222222-2222-2222-2222-222222222222"
                    .to_string(),
            ),
            logs: Some(
                "arn:aws:kms:us-east-1:123456789012:key/33333333-3333-3333-3333-333333333333"
                    .to_string(),
            ),
        }))
    }

    fn no_kms_keys() -> &'static KmsKeys {
        Box::leak(Box::new(KmsKeys::default()))
    }

    #[test]
    fn test_aws_session_kms_actions_follow_effective_s3_access() {
        for mode in [
            CredentialsAccess::ReadAppContent,
            CredentialsAccess::ReadAppDb,
            CredentialsAccess::ReadUser,
            CredentialsAccess::ReadLogs,
        ] {
            assert_eq!(
                kms_session_actions(&mode, false),
                &["kms:Decrypt"],
                "{mode} must not receive a data-key generation permission"
            );
        }

        for mode in [
            CredentialsAccess::EditApp,
            CredentialsAccess::EditAppContent,
            CredentialsAccess::EditAppDb,
            CredentialsAccess::EditUser,
            CredentialsAccess::InvokeNone,
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
            CredentialsAccess::ServerExecute,
        ] {
            assert_eq!(
                kms_session_actions(&mode, false),
                &["kms:Decrypt", "kms:GenerateDataKey"],
                "{mode} writes S3 objects and must be able to wrap a data key"
            );
        }

        assert_eq!(
            kms_session_actions(&CredentialsAccess::ReadApp, false),
            &["kms:Decrypt"],
            "standard-bucket ReadApp is a pure read"
        );
        assert_eq!(
            kms_session_actions(&CredentialsAccess::ReadApp, true),
            &["kms:Decrypt", "kms:GenerateDataKey"],
            "an SSE-KMS S3 Express ReadOnly session still requires both KMS actions"
        );
    }

    /// A shadow run never touches app or user content, but it does write
    /// scratch and append run logs — treating it as read-only would deny both
    /// on a CMK-encrypted bucket.
    #[test]
    fn test_aws_shadow_execute_can_wrap_data_keys() {
        assert_eq!(
            kms_session_actions(&CredentialsAccess::ShadowExecute, false),
            &["kms:Decrypt", "kms:GenerateDataKey"]
        );
    }

    /// Unpinned is the default: the assumed role's own policy is the boundary,
    /// and restating the key ARNs here would only spend policy budget.
    #[test]
    fn test_aws_unpinned_kms_statement_is_a_compact_role_ceiling() {
        let mode = CredentialsAccess::ReadUser;
        let statement = kms_statement(
            no_kms_keys(),
            &mode,
            kms_session_actions(&mode, false),
            "us-east-1",
            false,
        );

        assert_eq!(statement["Effect"], "Allow");
        assert_eq!(statement["Action"], json!(["kms:Decrypt"]));
        assert_eq!(statement["Resource"], "*");
        assert!(
            statement.get("Condition").is_none(),
            "an unpinned grant carries no condition: {statement}"
        );
    }

    /// With the ARNs configured the statement narrows to those keys and fences
    /// itself so a leaked session cannot call KMS except through S3.
    #[test]
    fn test_aws_pinned_kms_statement_names_keys_and_fences_via_service() {
        let mode = CredentialsAccess::ServerExecute;
        let statement = kms_statement(
            test_kms_keys(),
            &mode,
            kms_session_actions(&mode, false),
            "eu-west-1",
            true,
        );

        let resources = statement["Resource"].to_string();
        assert!(resources.contains("key/22222222"), "{resources}");
        assert!(resources.contains("key/33333333"), "{resources}");
        // The executor never opens the meta bucket, so its key stays unnamed.
        assert!(!resources.contains("key/11111111"), "{resources}");
        let via_service = statement["Condition"]["StringEquals"]["kms:ViaService"].to_string();
        assert!(
            via_service.contains("s3.eu-west-1.amazonaws.com"),
            "{via_service}"
        );
        assert!(
            via_service.contains("s3express.eu-west-1.amazonaws.com"),
            "{via_service}"
        );
    }

    /// The pinned resources must track the S3 statements: a mode that cannot
    /// read the meta bucket has no business holding its key.
    #[test]
    fn test_aws_pinned_kms_resources_follow_the_bucket_scope() {
        let keys = test_kms_keys();
        let meta_key = keys.meta.clone().expect("meta key");
        let logs_key = keys.logs.clone().expect("logs key");

        let content_key = keys.content.clone().expect("content key");

        let content_only = scoped_kms_resources(keys, &CredentialsAccess::EditAppContent)
            .expect("the content bucket is pinned");
        assert!(!content_only.contains(&meta_key.as_str()));
        assert!(!content_only.contains(&logs_key.as_str()));

        // `ReadLogs` reads the logs bucket and nothing else.
        let read_logs = scoped_kms_resources(keys, &CredentialsAccess::ReadLogs)
            .expect("the logs bucket is pinned");
        assert_eq!(read_logs, vec![logs_key.as_str()]);

        // The executor reads content and writes logs; boards arrive as presigned
        // artifacts, so the meta bucket and its key stay out of reach.
        let executor = scoped_kms_resources(keys, &CredentialsAccess::ServerExecute)
            .expect("the content and logs buckets are pinned");
        assert!(!executor.contains(&meta_key.as_str()));
        assert!(executor.contains(&content_key.as_str()));
        assert!(executor.contains(&logs_key.as_str()));
    }

    /// One key for all three buckets is the common setup; it must not appear
    /// three times in the resource list.
    #[test]
    fn test_aws_pinned_kms_resources_are_deduplicated() {
        let key = "arn:aws:kms:us-east-1:123456789012:key/11111111-1111-1111-1111-111111111111";
        let keys: &'static KmsKeys = Box::leak(Box::new(KmsKeys {
            meta: Some(key.to_string()),
            content: Some(key.to_string()),
            logs: Some(key.to_string()),
        }));

        assert_eq!(
            scoped_kms_resources(keys, &CredentialsAccess::ServerExecute),
            Some(vec![key])
        );
    }

    /// The KMS statement rides on top of the largest S3 policies; both the
    /// compact and the pinned form must fit the STS cap with realistic ids.
    #[test]
    fn test_aws_kms_statement_fits_the_policy_budget() {
        let creds = test_aws_runtime_credentials();
        let app = "apps/v8f5p73w00itor03zlrai22w";
        let user = "users/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let runs = "runs/v8f5p73w00itor03zlrai22w";
        let tmp_user = "tmp/user/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let tmp_global = "tmp/global/apps/v8f5p73w00itor03zlrai22w";

        for express in [true, false] {
            for keys in [no_kms_keys(), test_kms_keys()] {
                for mode in [
                    CredentialsAccess::ServerExecute,
                    CredentialsAccess::ShadowExecute,
                ] {
                    let build = if matches!(mode, CredentialsAccess::ServerExecute) {
                        server_execute_policy
                    } else {
                        shadow_execute_policy
                    };
                    let mut policy = build(&creds, app, user, runs, tmp_user, tmp_global);
                    policy["Statement"]
                        .as_array_mut()
                        .expect("statements")
                        .push(kms_statement(
                            keys,
                            &mode,
                            kms_session_actions(&mode, express),
                            "eu-central-1",
                            express,
                        ));

                    let json = to_string(&policy).expect("policy should serialize");
                    assert!(
                        json.len() <= STS_POLICY_MAX_CHARS,
                        "{mode} policy with KMS grants grew to {} chars",
                        json.len()
                    );
                }
            }
        }
    }

    /// Run logs are written to the logs bucket by `ServerExecute`; `ReadLogs`
    /// pointed at the content bucket and read a prefix nothing writes.
    #[test]
    fn test_aws_read_logs_uses_only_the_logs_bucket() {
        let policy = to_string(&read_logs_policy(
            &test_aws_runtime_credentials(),
            "runs/app-1",
        ))
        .expect("policy should serialize");

        assert!(policy.contains("arn:aws:s3:::logs"));
        assert!(policy.contains("arn:aws:s3:::logs/runs/app-1/*"));
        assert!(
            !policy.contains("content-data"),
            "ReadLogs must not target the content bucket: {policy}"
        );
    }

    #[test]
    fn test_aws_runtime_credentials_serialization() {
        let creds = AwsRuntimeCredentials {
            access_key_id: Some("AKIATEST".to_string()),
            secret_access_key: Some("secret".to_string()),
            session_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            region: "us-east-1".to_string(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");
        let deserialized: AwsRuntimeCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.access_key_id, deserialized.access_key_id);
        assert_eq!(creds.region, deserialized.region);
    }

    #[test]
    fn test_credentials_access_display() {
        use crate::credentials::CredentialsAccess;

        assert_eq!(format!("{}", CredentialsAccess::EditApp), "edit_app");
        assert_eq!(format!("{}", CredentialsAccess::ReadApp), "read_app");
        assert_eq!(format!("{}", CredentialsAccess::InvokeNone), "invoke_none");
        assert_eq!(format!("{}", CredentialsAccess::InvokeRead), "invoke_read");
        assert_eq!(
            format!("{}", CredentialsAccess::InvokeWrite),
            "invoke_write"
        );
        assert_eq!(
            format!("{}", CredentialsAccess::ServerExecute),
            "server_execute"
        );
        assert_eq!(format!("{}", CredentialsAccess::ReadLogs), "read_logs");
    }
}
