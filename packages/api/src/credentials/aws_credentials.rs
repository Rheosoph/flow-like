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

        let role = std::env::var("RUNTIME_ROLE_ARN").map_err(|_| {
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
                meta_bucket_express_zone(),
            ),
            CredentialsAccess::ShadowExecute => shadow_execute_policy(
                self,
                &apps_prefix,
                &user_prefix,
                &runs_prefix,
                &temporary_user_prefix,
                &temporary_global_prefix,
                meta_bucket_express_zone(),
            ),
            CredentialsAccess::ReadLogs => read_logs_policy(self, &runs_prefix),
        };

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

/// Whether the meta bucket is an S3 Express directory bucket. On a directory
/// bucket the CreateSession token — not per-object IAM — authorizes every
/// zonal request, so the two bucket flavors need disjoint policy statements.
/// Emitting both flavors in one session policy pushed AssumeRole past its
/// packed-policy budget (PackedPolicyTooLargeException at dispatch).
/// Deployment configuration never changes within a process, so the env var is
/// read once.
fn meta_bucket_express_zone() -> bool {
    static META_BUCKET_EXPRESS_ZONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *META_BUCKET_EXPRESS_ZONE.get_or_init(|| {
        std::env::var("META_BUCKET_EXPRESS_ZONE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false)
    })
}

#[cfg(feature = "aws")]
#[async_trait]
impl RuntimeCredentialsTrait for AwsRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        let meta_express = meta_bucket_express_zone();
        let content_express = std::env::var("CONTENT_BUCKET_EXPRESS_ZONE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let logs_express = std::env::var("LOGS_BUCKET_EXPRESS_ZONE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let meta_endpoint = std::env::var("META_BUCKET_ENDPOINT").ok();
        let content_endpoint = std::env::var("CONTENT_BUCKET_ENDPOINT").ok();
        let logs_endpoint = std::env::var("LOGS_BUCKET_ENDPOINT").ok();

        // Build optional configs only if there's something to configure
        let meta_config = if meta_endpoint.is_some() || meta_express {
            Some(BucketConfig {
                endpoint: meta_endpoint,
                express: meta_express,
            })
        } else {
            None
        };
        let content_config = if content_endpoint.is_some() || content_express {
            Some(BucketConfig {
                endpoint: content_endpoint,
                express: content_express,
            })
        } else {
            None
        };
        let logs_config = if logs_endpoint.is_some() || logs_express {
            Some(BucketConfig {
                endpoint: logs_endpoint,
                express: logs_express,
            })
        } else {
            None
        };

        SharedCredentials::Aws(AwsSharedCredentials {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            meta_config,
            content_config,
            logs_config,
            region: self.region.clone(),
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
        })
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

        let (bkt, key, secret, token) = (
            self.content_bucket.clone(),
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

        config.register_build_logs_database(Arc::new(make_s3_builder(
            bkt.clone(),
            key.clone(),
            secret.clone(),
            token.clone(),
        )));
        config.register_build_project_database(Arc::new(make_s3_builder(
            bkt.clone(),
            key.clone(),
            secret.clone(),
            token.clone(),
        )));
        config.register_build_user_database(Arc::new(make_s3_builder(bkt, key, secret, token)));

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
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("s3://{}/{}", bucket, path);
        connect(&url)
            .storage_option("aws_access_key_id".to_string(), access_key.clone())
            .storage_option("aws_secret_access_key".to_string(), secret_key.clone())
            .storage_option("aws_session_token".to_string(), session_token.clone())
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

fn server_execute_policy(
    credentials: &AwsRuntimeCredentials,
    apps_prefix: &str,
    user_prefix: &str,
    runs_prefix: &str,
    temporary_user_prefix: &str,
    temporary_global_prefix: &str,
    meta_express: bool,
) -> flow_like_types::Value {
    // Draft artifacts (compiled boards, exact `.board` snapshots, prerun
    // manifests) live under `tmp/apps/{app_id}/` on the meta bucket so
    // lifecycle rules on `tmp/` reclaim them. The executor gets the same
    // read-only grant there as on `apps/{app_id}` — only the API writes
    // drafts, which anchors `entry_authority_revision`.
    let draft_meta_prefix = format!("tmp/{apps_prefix}");
    let mut statements = vec![
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
    if meta_express {
        // On a directory bucket the session token — not per-object IAM —
        // authorizes every zonal request, so the `s3:*` meta statements would
        // never be evaluated; emitting them anyway blew the AssumeRole packed
        // policy budget. Pin the session to ReadOnly so this mode cannot
        // write the meta bucket.
        statements.push(json!({
          "Effect": "Allow",
          "Action": [
              "s3express:CreateSession",
          ],
          "Resource": [
              "*"
          ],
          "Condition": {
              "StringEquals": {
                  "s3express:SessionMode": "ReadOnly"
              }
          }
        }));
    } else {
        statements.push(json!({
          "Effect": "Allow",
          "Action": [
              "s3:ListBucket"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}", credentials.meta_bucket)
          ],
          "Condition": {
              "StringLike": {
                  "s3:prefix": [
                      format!("{}/*", apps_prefix),
                      format!("{}/*", draft_meta_prefix)
                  ]
              }
          }
        }));
        statements.push(json!({
          "Effect": "Allow",
          "Action": [
              "s3:GetObject"
          ],
          "Resource": [
              format!("arn:aws:s3:::{}/{}/*", credentials.meta_bucket, apps_prefix),
              format!("arn:aws:s3:::{}/{}/*", credentials.meta_bucket, draft_meta_prefix),
          ],
        }));
    }
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
    meta_express: bool,
) -> flow_like_types::Value {
    let mut policy = server_execute_policy(
        credentials,
        apps_prefix,
        user_prefix,
        runs_prefix,
        temporary_user_prefix,
        temporary_global_prefix,
        meta_express,
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
                format!("arn:aws:s3:::{}", credentials.content_bucket),
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
                format!("arn:aws:s3:::{}/{}/*", credentials.content_bucket, runs_prefix),
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
    fn test_aws_server_execute_grants_meta_read_only() {
        let creds = test_aws_runtime_credentials();
        let app_prefix = "apps/app-1";
        let policy = server_execute_policy(
            &creds,
            app_prefix,
            "users/user-1/apps/app-1",
            "runs/app-1",
            "tmp/user/user-1/apps/app-1",
            "tmp/global/apps/app-1",
            false,
        );
        let statements = policy["Statement"]
            .as_array()
            .expect("policy statements should be an array");
        let meta_object_statement = statements
            .iter()
            .find(|statement| {
                statement["Resource"]
                    .to_string()
                    .contains("meta-secret/apps")
            })
            .expect("server execute should include meta object access");
        let meta_list_statement = statements
            .iter()
            .find(|statement| {
                statement["Resource"]
                    .to_string()
                    .contains("arn:aws:s3:::meta-secret")
                    && statement["Condition"].to_string().contains(app_prefix)
            })
            .expect("server execute should include app-scoped meta listing");
        let meta_actions = meta_object_statement["Action"].to_string();
        let meta_resources = meta_object_statement["Resource"].to_string();
        let meta_list_actions = meta_list_statement["Action"].to_string();
        let meta_list_prefixes = meta_list_statement["Condition"].to_string();
        for statement in statements {
            let actions = statement["Action"].to_string();
            if statement["Resource"].to_string().contains("meta-secret") {
                assert!(
                    !actions.contains("PutObject") && !actions.contains("DeleteObject"),
                    "no ServerExecute statement may write the meta bucket: {statement}"
                );
            }
        }
        let policy = to_string(&policy).expect("policy should serialize");
        let draft_artifact =
            flow_like::flow::compiled::draft_artifact_path("app-1", "board-1", "etag", &[7; 32])
                .to_string();
        let other_app_artifact =
            flow_like::flow::compiled::draft_artifact_path("app-10", "board-1", "etag", &[7; 32])
                .to_string();

        assert!(policy.contains("meta-secret/apps/app-1/*"));
        assert!(meta_actions.contains("GetObject"));
        assert!(meta_list_actions.contains("ListBucket"));
        assert!(draft_artifact.starts_with("tmp/apps/app-1/"));
        assert!(!other_app_artifact.starts_with("tmp/apps/app-1/"));
        assert!(
            meta_resources.contains("meta-secret/tmp/apps/app-1/*"),
            "draft artifacts under tmp/apps must stay readable: {meta_resources}"
        );
        assert!(
            meta_list_prefixes.contains("tmp/apps/app-1/*"),
            "draft artifacts under tmp/apps must stay listable: {meta_list_prefixes}"
        );
        assert!(
            !policy.to_string().contains("s3express"),
            "a standard meta bucket needs no express statements"
        );
        assert!(policy.contains("content-data/apps/app-1/*"));
        assert!(policy.contains("logs/runs/app-1/*"));
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
        assert!(
            !meta_actions.contains("PutObject"),
            "ServerExecute must not grant meta writes"
        );
        assert!(
            !meta_actions.contains("DeleteObject"),
            "ServerExecute must not grant meta deletes"
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
            false,
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

        for express in [true, false] {
            let policy =
                shadow_execute_policy(&creds, app, user, runs, tmp_user, tmp_global, express);
            let json = to_string(&policy).expect("policy should serialize");
            assert!(
                json.len() < 2000,
                "{} ShadowExecute policy grew to {} chars",
                if express { "express" } else { "standard" },
                json.len()
            );
        }
    }

    /// On an express meta bucket the CreateSession token authorizes every
    /// zonal request, so the per-object meta statements are dead weight — and
    /// carrying both flavors once pushed AssumeRole past its packed-policy
    /// budget in production (PackedPolicyTooLargeException at dispatch).
    #[test]
    fn test_aws_server_execute_express_variant_stays_within_policy_budget() {
        let creds = test_aws_runtime_credentials();
        // Realistic id lengths: generated app ids and IdP subjects are long,
        // and every prefix repeats them several times.
        let app = "apps/v8f5p73w00itor03zlrai22w";
        let user = "users/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let runs = "runs/v8f5p73w00itor03zlrai22w";
        let tmp_user = "tmp/user/auth0|64c9f0aa11bb22cc33dd44ee/apps/v8f5p73w00itor03zlrai22w";
        let tmp_global = "tmp/global/apps/v8f5p73w00itor03zlrai22w";

        let express = server_execute_policy(&creds, app, user, runs, tmp_user, tmp_global, true);
        let express_json = to_string(&express).expect("policy should serialize");
        assert!(
            !express_json.contains("arn:aws:s3:::meta-secret"),
            "express meta access flows through CreateSession, not per-object IAM"
        );
        assert!(express_json.contains("s3express:CreateSession"));
        assert!(express_json.contains("ReadOnly"));

        let standard = server_execute_policy(&creds, app, user, runs, tmp_user, tmp_global, false);
        let standard_json = to_string(&standard).expect("policy should serialize");

        // STS caps the plaintext at 2048 characters and packs it into a
        // shared binary quota; both variants must keep real headroom.
        assert!(
            express_json.len() < 1600,
            "express ServerExecute policy grew to {} chars",
            express_json.len()
        );
        assert!(
            standard_json.len() < 1900,
            "standard ServerExecute policy grew to {} chars",
            standard_json.len()
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
                true,
            ))
            .as_deref(),
            Some("\"ReadOnly\""),
            "ServerExecute only reads the meta bucket and must not obtain a ReadWrite session"
        );
        assert_eq!(
            create_session_mode(edit_app_policy(&creds, apps_prefix)).as_deref(),
            Some("null"),
            "EditApp legitimately writes the meta bucket"
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
