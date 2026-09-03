//! Which concrete bucket each object store on [`crate::state::State`] points at.
//!
//! A `FlowLikeStore` is an opaque `ObjectStore` handle: it can read and write, but it
//! cannot say which bucket, region or account it is bound to. Every provider metrics
//! API needs exactly that — CloudWatch takes a `BucketName` dimension in the bucket's
//! own region, Azure Monitor takes an ARM resource id built from the storage account,
//! Cloudflare takes an `accountTag`. Resolving it once from the master credentials keeps
//! the answer in step with the stores those same credentials built.

use crate::credentials::RuntimeCredentials;

/// Storage family behind a bucket.
///
/// Distinct from [`crate::storage_config::StorageProvider`], which only names the three
/// providers that module can *build* a store for. Here `R2` has to stay separate from
/// `Aws` even though both speak S3, because their metrics APIs share nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StorageProviderKind {
    Aws,
    Azure,
    Gcp,
    R2,
    Local,
    Memory,
    #[default]
    Other,
}

impl StorageProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Azure => "azure",
            Self::Gcp => "gcp",
            Self::R2 => "r2",
            Self::Local => "local",
            Self::Memory => "memory",
            Self::Other => "other",
        }
    }
}

impl std::fmt::Display for StorageProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything a metrics API needs to name one bucket.
#[derive(Clone, Debug, Default)]
pub struct BucketIdentity {
    pub provider: StorageProviderKind,
    /// Bucket or container name. Empty when the deployment configured none.
    pub name: String,
    /// AWS region. CloudWatch serves a bucket's storage metrics only from the region the
    /// bucket lives in, and answers a query aimed elsewhere with an empty result set
    /// rather than an error.
    pub region: Option<String>,
    /// Azure storage account, or the Cloudflare account id for R2.
    pub account: Option<String>,
    /// Set only for S3-compatible stores that are not AWS S3 (MinIO, Ceph, R2 over the
    /// S3 API). Its presence is what disqualifies the CloudWatch path.
    pub endpoint: Option<String>,
}

impl BucketIdentity {
    /// Human-readable identity for the dashboard card.
    pub fn describe(&self) -> String {
        let mut qualifiers = Vec::new();
        if let Some(region) = &self.region {
            qualifiers.push(format!("region {region}"));
        }
        if let Some(account) = &self.account {
            qualifiers.push(format!("account {account}"));
        }
        if let Some(endpoint) = &self.endpoint {
            qualifiers.push(endpoint.clone());
        }

        let name = if self.name.is_empty() {
            "no bucket configured"
        } else {
            self.name.as_str()
        };

        if qualifiers.is_empty() {
            name.to_string()
        } else {
            format!("{name} — {}", qualifiers.join(", "))
        }
    }
}

/// Identity of every bucket this deployment serves from.
#[derive(Clone, Debug, Default)]
pub struct StorageIdentity {
    pub meta: BucketIdentity,
    pub content: BucketIdentity,
    pub cdn: BucketIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BucketRole {
    Meta,
    Content,
}

/// Resolve the identity of all three buckets from the deployment's master credentials.
pub fn from_credentials(credentials: &RuntimeCredentials) -> StorageIdentity {
    let meta = identity_for(credentials, BucketRole::Meta);
    let content = identity_for(credentials, BucketRole::Content);
    let cdn = cdn_identity(&content);

    StorageIdentity { meta, content, cdn }
}

/// Resolve one bucket.
///
/// A mixed deployment holds a whole credential set per bucket, so the role has to be
/// pushed down rather than resolved once: its meta bucket may be on a different provider
/// — with a different region and account — from its content bucket.
fn identity_for(credentials: &RuntimeCredentials, role: BucketRole) -> BucketIdentity {
    match credentials {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(aws) => BucketIdentity {
            provider: StorageProviderKind::Aws,
            name: match role {
                BucketRole::Meta => aws.meta_bucket.clone(),
                BucketRole::Content => aws.content_bucket.clone(),
            },
            region: non_empty(&aws.region),
            account: None,
            endpoint: env_non_empty("AWS_ENDPOINT"),
        },
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(azure) => BucketIdentity {
            provider: StorageProviderKind::Azure,
            name: match role {
                BucketRole::Meta => azure.meta_container.clone(),
                BucketRole::Content => azure.content_container.clone(),
            },
            region: None,
            account: non_empty(&azure.account_name),
            endpoint: None,
        },
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(gcp) => BucketIdentity {
            provider: StorageProviderKind::Gcp,
            name: match role {
                BucketRole::Meta => gcp.meta_bucket.clone(),
                BucketRole::Content => gcp.content_bucket.clone(),
            },
            region: None,
            account: None,
            endpoint: None,
        },
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(r2) => BucketIdentity {
            provider: StorageProviderKind::R2,
            name: match role {
                BucketRole::Meta => r2.meta_bucket.clone(),
                BucketRole::Content => r2.content_bucket.clone(),
            },
            region: None,
            account: non_empty(&r2.account_id),
            endpoint: non_empty(&r2.endpoint),
        },
        RuntimeCredentials::Mixed(mixed) => match role {
            BucketRole::Meta => identity_for(mixed.meta.as_ref(), role),
            BucketRole::Content => identity_for(mixed.content.as_ref(), role),
        },
    }
}

/// The CDN bucket is not part of `RuntimeCredentials` — it is only ever named by
/// environment, and `BucketConfig::from_env` falls back to the content bucket when it is
/// unset. Mirroring that fallback here matters: a single-bucket deployment must show the
/// CDN card pointing at the same bucket it actually serves from, not at nothing.
fn cdn_identity(content: &BucketIdentity) -> BucketIdentity {
    let provider_specific = match content.provider {
        StorageProviderKind::Aws => Some("AWS_CDN_BUCKET"),
        StorageProviderKind::Azure => Some("AZURE_CDN_CONTAINER"),
        StorageProviderKind::Gcp => Some("GCP_CDN_BUCKET"),
        _ => None,
    };

    let name = env_non_empty("CDN_BUCKET_NAME")
        .or_else(|| provider_specific.and_then(env_non_empty))
        .unwrap_or_else(|| content.name.clone());

    BucketIdentity {
        name,
        ..content.clone()
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().as_deref().and_then(non_empty)
}
