//! Audit keys held in a key service, selected by `AUDIT_KMS_KEY_ID`.
//!
//! Every backend signs the 32-byte digest as the message of ECDSA P-256 with SHA-256,
//! which is exactly what [`super::signer::LocalSigner`] produces, and every signature is
//! checked against the key's public half before it is returned. The provider comes from
//! `AUDIT_KMS_PROVIDER` or, when unset, from the shape of the key id: `projects/...` is
//! Cloud KMS, `https://...` is Azure Key Vault, anything else AWS KMS — except that a
//! set `AUDIT_VAULT_ADDR` selects Vault or OpenBao, the self-hosted option that needs no
//! cloud account and is always compiled in.

use super::signer::SharedSigner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Aws,
    Gcp,
    Azure,
    /// HashiCorp Vault or OpenBao `transit`, self-hosted anywhere.
    Vault,
}

impl Provider {
    fn resolve(
        explicit: Option<&str>,
        key_id: &str,
        vault_address: Option<&str>,
    ) -> flow_like_types::Result<Self> {
        match explicit.map(str::to_ascii_lowercase).as_deref() {
            Some("aws") => Ok(Self::Aws),
            Some("gcp" | "google") => Ok(Self::Gcp),
            Some("azure") => Ok(Self::Azure),
            Some("vault" | "openbao" | "transit") => Ok(Self::Vault),
            Some(other) => Err(flow_like_types::anyhow!(
                "AUDIT_KMS_PROVIDER must be aws, gcp, azure or vault, got {other:?}"
            )),
            None if key_id.starts_with("projects/") => Ok(Self::Gcp),
            None if key_id.starts_with("https://") => Ok(Self::Azure),
            None if vault_address.is_some() => Ok(Self::Vault),
            None => Ok(Self::Aws),
        }
    }

    fn feature(self) -> &'static str {
        match self {
            Self::Aws => "audit-kms-aws",
            Self::Gcp => "audit-kms-gcp",
            Self::Azure => "audit-kms-azure",
            Self::Vault => "audit-kms-vault",
        }
    }

    fn compiled(self) -> bool {
        match self {
            Self::Aws => cfg!(feature = "audit-kms-aws"),
            Self::Gcp => cfg!(feature = "audit-kms-gcp"),
            Self::Azure => cfg!(feature = "audit-kms-azure"),
            // Plain HTTP; no cloud SDK to compile in.
            Self::Vault => true,
        }
    }
}

/// `AUDIT_VAULT_ADDR`, else `VAULT_ADDR`.
fn vault_address() -> Option<String> {
    crate::storage_config::non_empty_env("AUDIT_VAULT_ADDR")
        .or_else(|| crate::storage_config::non_empty_env("VAULT_ADDR"))
}

/// Whether `AUDIT_KMS_KEY_ID` names a key this build can use. Makes no request; a key
/// for a provider this build lacks is an error that names the feature.
pub fn configured() -> flow_like_types::Result<bool> {
    let Some(key_id) = crate::storage_config::non_empty_env("AUDIT_KMS_KEY_ID") else {
        return Ok(false);
    };
    let explicit = crate::storage_config::non_empty_env("AUDIT_KMS_PROVIDER");
    let provider = Provider::resolve(explicit.as_deref(), &key_id, vault_address().as_deref())?;
    if !provider.compiled() {
        return Err(flow_like_types::anyhow!(
            "AUDIT_KMS_KEY_ID {key_id} needs the `{}` feature of flow-like-api, which this build lacks",
            provider.feature()
        ));
    }
    Ok(true)
}

/// The signer for `AUDIT_KMS_KEY_ID`, or `None` when it is unset. `kid` defaults to the
/// public key's fingerprint, like the local key.
///
/// Only the audit worker calls this. Every request to a key service is billed, so the
/// public key is read from the service only when `AUDIT_KID` does not already name a
/// key registered from `AUDIT_VERIFYING_KEYS`; the first signature is then checked
/// against that registered key. Vault still reads its key metadata once to pin the
/// version whose public key matches the registered key.
#[cfg_attr(
    not(any(
        feature = "audit-kms-aws",
        feature = "audit-kms-gcp",
        feature = "audit-kms-azure"
    )),
    allow(unused_variables, unreachable_code)
)]
pub async fn from_env(kid: Option<String>) -> flow_like_types::Result<Option<SharedSigner>> {
    let Some(key_id) = crate::storage_config::non_empty_env("AUDIT_KMS_KEY_ID") else {
        return Ok(None);
    };
    let explicit = crate::storage_config::non_empty_env("AUDIT_KMS_PROVIDER");
    let provider = Provider::resolve(explicit.as_deref(), &key_id, vault_address().as_deref())?;
    let known = kid.as_deref().and_then(super::signer::registered_key);
    let signer: SharedSigner = match provider {
        Provider::Vault => remote::signer(vault::Vault::connect(key_id, known).await?, kid),
        #[cfg(feature = "audit-kms-aws")]
        Provider::Aws => remote::signer(aws::Aws::connect(key_id, known).await?, kid),
        #[cfg(feature = "audit-kms-gcp")]
        Provider::Gcp => remote::signer(gcp::Gcp::connect(key_id, known).await?, kid),
        #[cfg(feature = "audit-kms-azure")]
        Provider::Azure => remote::signer(azure::Azure::connect(key_id, known).await?, kid),
        #[allow(unreachable_patterns)]
        other => {
            return Err(flow_like_types::anyhow!(
                "AUDIT_KMS_KEY_ID {key_id} needs the `{}` feature of flow-like-api, which this build lacks",
                other.feature()
            ));
        }
    };
    Ok(Some(signer))
}

mod remote {
    use std::sync::Arc;

    use flow_like_types::async_trait;
    use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};

    use crate::audit::crypto::Hash;
    use crate::audit::signer::{AuditSigner, RawSignature, SharedSigner, fingerprint_kid};

    #[async_trait]
    pub(super) trait Backend: Send + Sync + 'static {
        /// Key id as configured, for error messages.
        fn key_id(&self) -> &str;
        fn public_key(&self) -> VerifyingKey;
        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature>;
    }

    struct KmsSigner<B> {
        backend: B,
        kid: String,
        key: VerifyingKey,
    }

    pub(super) fn signer<B: Backend>(backend: B, kid: Option<String>) -> SharedSigner {
        let key = backend.public_key();
        let kid = kid
            .map(|kid| kid.trim().to_owned())
            .filter(|kid| !kid.is_empty())
            .unwrap_or_else(|| fingerprint_kid(&key));
        Arc::new(KmsSigner { backend, kid, key })
    }

    #[async_trait]
    impl<B: Backend> AuditSigner for KmsSigner<B> {
        fn kid(&self) -> &str {
            &self.kid
        }

        fn verifying_key(&self) -> VerifyingKey {
            self.key
        }

        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            let raw = self.backend.sign(digest).await?;
            let signature = Signature::from_slice(&raw).map_err(|_| {
                flow_like_types::anyhow!(
                    "key service returned a malformed signature for {}",
                    self.backend.key_id()
                )
            })?;
            self.key.verify(digest, &signature).map_err(|_| {
                flow_like_types::anyhow!(
                    "signature from {} does not verify under its public key",
                    self.backend.key_id()
                )
            })?;
            Ok(raw)
        }
    }

    /// Raw `r || s` in low-S form, the form [`crate::audit::signer::raw_from_der`] returns.
    #[cfg_attr(not(feature = "audit-kms-azure"), allow(dead_code))]
    pub(super) fn raw_low_s(raw: &[u8], key_id: &str) -> flow_like_types::Result<RawSignature> {
        let signature = Signature::from_slice(raw).map_err(|_| {
            flow_like_types::anyhow!(
                "key service returned {} signature bytes for {key_id}, expected 64",
                raw.len()
            )
        })?;
        let signature = signature.normalize_s().unwrap_or(signature);
        Ok(signature.to_bytes().into())
    }

    // Vault, Cloud KMS and Key Vault all speak HTTP JSON.
    #[cfg_attr(
        not(any(feature = "audit-kms-gcp", feature = "audit-kms-azure")),
        allow(dead_code)
    )]
    pub(super) fn http_client() -> flow_like_types::Result<reqwest::Client> {
        client(true, None)
    }

    /// `https_only` is relaxed only for a Vault reached inside a private network.
    /// `extra_roots` is a PEM bundle trusted in addition to the built-in roots, which
    /// a self-hosted key service signed by a private CA needs to be reachable at all.
    pub(super) fn client(
        https_only: bool,
        extra_roots: Option<&[u8]>,
    ) -> flow_like_types::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .https_only(https_only)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("flow-like-audit/", env!("CARGO_PKG_VERSION")));
        if let Some(pem) = extra_roots {
            let roots = reqwest::Certificate::from_pem_bundle(pem).map_err(|error| {
                flow_like_types::anyhow!("the audit key service CA bundle is not PEM: {error}")
            })?;
            if roots.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "the audit key service CA bundle holds no certificate"
                ));
            }
            for root in roots {
                builder = builder.add_root_certificate(root);
            }
        }
        builder.build().map_err(|error| {
            flow_like_types::anyhow!("building the audit key service client failed: {error}")
        })
    }

    /// Parse a JSON success body, or name the operation and status of a failure.
    pub(super) async fn json_body<T: serde::de::DeserializeOwned>(
        response: reqwest::Result<reqwest::Response>,
        operation: &str,
    ) -> flow_like_types::Result<T> {
        const MAX_ERROR_CHARS: usize = 300;
        let response =
            response.map_err(|error| flow_like_types::anyhow!("{operation} failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(MAX_ERROR_CHARS).collect();
            return Err(flow_like_types::anyhow!(
                "{operation} failed with HTTP {status}: {body}"
            ));
        }
        response.json::<T>().await.map_err(|error| {
            flow_like_types::anyhow!("{operation} returned an unexpected body: {error}")
        })
    }
}

#[cfg(feature = "audit-kms-aws")]
mod aws {
    use aws_sdk_kms::config::retry::RetryConfig;
    use aws_sdk_kms::error::DisplayErrorContext;
    use aws_sdk_kms::primitives::Blob;
    use aws_sdk_kms::types::{KeySpec, MessageType, SigningAlgorithmSpec};
    use flow_like_types::async_trait;
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::DecodePublicKey;

    use super::remote::Backend;
    use crate::audit::crypto::Hash;
    use crate::audit::signer::{RawSignature, raw_from_der};

    pub(super) struct Aws {
        client: aws_sdk_kms::Client,
        key_id: String,
        key: VerifyingKey,
    }

    /// Each attempt is a billed request; one retry covers a transient failure.
    const MAX_ATTEMPTS: u32 = 2;

    /// The KMS client's own region and credentials. Self-hosted setups set
    /// `AWS_ACCESS_KEY_ID` to their object store's identity, which the default chain
    /// would otherwise pick up: `AUDIT_KMS_AWS_ACCESS_KEY_ID` and
    /// `AUDIT_KMS_AWS_SECRET_ACCESS_KEY` take precedence, and the region comes from
    /// `AUDIT_KMS_REGION` or the key ARN before the default chain.
    fn sdk_config(key_id: &str) -> flow_like_types::Result<aws_config::ConfigLoader> {
        use crate::storage_config::non_empty_env;
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(region) = non_empty_env("AUDIT_KMS_REGION").or_else(|| arn_region(key_id)) {
            loader = loader.region(aws_config::Region::new(region));
        }
        match (
            non_empty_env("AUDIT_KMS_AWS_ACCESS_KEY_ID"),
            non_empty_env("AUDIT_KMS_AWS_SECRET_ACCESS_KEY"),
        ) {
            (Some(access_key), Some(secret_key)) => {
                loader = loader.credentials_provider(aws_sdk_kms::config::Credentials::new(
                    access_key,
                    secret_key,
                    None,
                    None,
                    "audit-kms",
                ));
            }
            (None, None) => {}
            _ => {
                return Err(flow_like_types::anyhow!(
                    "set both AUDIT_KMS_AWS_ACCESS_KEY_ID and AUDIT_KMS_AWS_SECRET_ACCESS_KEY, or neither"
                ));
            }
        }
        Ok(loader)
    }

    /// `arn:aws:kms:<region>:<account>:key/<id>` or an alias ARN.
    fn arn_region(key_id: &str) -> Option<String> {
        let mut parts = key_id.split(':');
        (parts.next() == Some("arn") && parts.nth(1) == Some("kms"))
            .then(|| parts.next())
            .flatten()
            .filter(|region| !region.is_empty())
            .map(str::to_owned)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn region_comes_from_key_arns_only() {
            assert_eq!(
                super::arn_region("arn:aws:kms:eu-central-1:123456789012:key/abcd").as_deref(),
                Some("eu-central-1")
            );
            assert_eq!(
                super::arn_region("arn:aws:kms:us-east-1:123456789012:alias/audit").as_deref(),
                Some("us-east-1")
            );
            assert_eq!(super::arn_region("alias/audit"), None);
            assert_eq!(
                super::arn_region("1234abcd-12ab-34cd-56ef-1234567890ab"),
                None
            );
        }
    }

    impl Aws {
        /// `key_id` is a key id, key ARN, alias name or alias ARN of an
        /// `ECC_NIST_P256` signing key. `known` skips reading the public key.
        pub(super) async fn connect(
            key_id: String,
            known: Option<VerifyingKey>,
        ) -> flow_like_types::Result<Self> {
            let config = sdk_config(&key_id)?.load().await;
            let config = aws_sdk_kms::config::Builder::from(&config)
                .retry_config(RetryConfig::standard().with_max_attempts(MAX_ATTEMPTS))
                .build();
            let client = aws_sdk_kms::Client::from_conf(config);
            if let Some(key) = known {
                return Ok(Self {
                    client,
                    key_id,
                    key,
                });
            }
            let output = client
                .get_public_key()
                .key_id(&key_id)
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "reading the public key of AWS KMS key {key_id} failed: {}",
                        DisplayErrorContext(&error)
                    )
                })?;
            if output.key_spec() != Some(&KeySpec::EccNistP256) {
                return Err(flow_like_types::anyhow!(
                    "AWS KMS key {key_id} must have key spec ECC_NIST_P256, got {:?}",
                    output.key_spec()
                ));
            }
            let der = output.public_key().ok_or_else(|| {
                flow_like_types::anyhow!("AWS KMS returned no public key for {key_id}")
            })?;
            let key = VerifyingKey::from_public_key_der(der.as_ref()).map_err(|_| {
                flow_like_types::anyhow!("public key of AWS KMS key {key_id} is not P-256 SPKI")
            })?;
            Ok(Self {
                client,
                key_id,
                key,
            })
        }
    }

    #[async_trait]
    impl Backend for Aws {
        fn key_id(&self) -> &str {
            &self.key_id
        }

        fn public_key(&self) -> VerifyingKey {
            self.key
        }

        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            let output = self
                .client
                .sign()
                .key_id(&self.key_id)
                .message(Blob::new(digest.to_vec()))
                .message_type(MessageType::Raw)
                .signing_algorithm(SigningAlgorithmSpec::EcdsaSha256)
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!(
                        "signing with AWS KMS key {} failed: {}",
                        self.key_id,
                        DisplayErrorContext(&error)
                    )
                })?;
            let der = output.signature().ok_or_else(|| {
                flow_like_types::anyhow!("AWS KMS returned no signature for {}", self.key_id)
            })?;
            raw_from_der(der.as_ref())
        }
    }
}

#[cfg(feature = "audit-kms-gcp")]
mod gcp {
    use flow_like_types::async_trait;
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::DecodePublicKey;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    use super::remote::{Backend, http_client, json_body};
    use crate::audit::crypto::Hash;
    use crate::audit::signer::{RawSignature, raw_from_der};
    use crate::credentials::gcp_credentials;

    const ENDPOINT: &str = "https://cloudkms.googleapis.com/v1";
    const SCOPE: &str = "https://www.googleapis.com/auth/cloudkms";
    const ALGORITHM: &str = "EC_SIGN_P256_SHA256";

    pub(super) struct Gcp {
        http: reqwest::Client,
        name: String,
        key: VerifyingKey,
    }

    #[derive(Deserialize)]
    struct PublicKey {
        pem: String,
        algorithm: String,
    }

    #[derive(Deserialize)]
    struct Signed {
        signature: String,
    }

    async fn token() -> flow_like_types::Result<String> {
        match gcp_credentials::service_account_key_from_env() {
            Some(key) => gcp_credentials::generate_access_token_standalone(&key, SCOPE).await,
            None => gcp_credentials::fetch_metadata_token().await,
        }
    }

    impl Gcp {
        /// `name` is a key version:
        /// `projects/P/locations/L/keyRings/R/cryptoKeys/K/cryptoKeyVersions/V`.
        /// `known` skips reading the public key.
        pub(super) async fn connect(
            name: String,
            known: Option<VerifyingKey>,
        ) -> flow_like_types::Result<Self> {
            if !name.starts_with("projects/") || !name.contains("/cryptoKeyVersions/") {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_KMS_KEY_ID {name} must name a Cloud KMS key version (projects/.../cryptoKeyVersions/N)"
                ));
            }
            let http = http_client()?;
            if let Some(key) = known {
                return Ok(Self { http, name, key });
            }
            let request = http
                .get(format!("{ENDPOINT}/{name}/publicKey"))
                .bearer_auth(token().await?)
                .send()
                .await;
            let public: PublicKey = json_body(
                request,
                &format!("reading the public key of Cloud KMS key {name}"),
            )
            .await?;
            if public.algorithm != ALGORITHM {
                return Err(flow_like_types::anyhow!(
                    "Cloud KMS key {name} must use {ALGORITHM}, got {}",
                    public.algorithm
                ));
            }
            let key = VerifyingKey::from_public_key_pem(&public.pem).map_err(|_| {
                flow_like_types::anyhow!("public key of Cloud KMS key {name} is not P-256 PEM")
            })?;
            Ok(Self { http, name, key })
        }
    }

    #[async_trait]
    impl Backend for Gcp {
        fn key_id(&self) -> &str {
            &self.name
        }

        fn public_key(&self) -> VerifyingKey {
            self.key
        }

        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            let body = serde_json::json!({
                "digest": { "sha256": STANDARD.encode(Sha256::digest(digest)) }
            });
            let request = self
                .http
                .post(format!("{ENDPOINT}/{}:asymmetricSign", self.name))
                .bearer_auth(token().await?)
                .json(&body)
                .send()
                .await;
            let signed: Signed = json_body(
                request,
                &format!("signing with Cloud KMS key {}", self.name),
            )
            .await?;
            let der = STANDARD.decode(signed.signature.as_bytes()).map_err(|_| {
                flow_like_types::anyhow!(
                    "Cloud KMS returned a signature for {} that is not base64",
                    self.name
                )
            })?;
            raw_from_der(&der)
        }
    }
}

#[cfg(feature = "audit-kms-azure")]
mod azure {
    use std::sync::Arc;

    use azure_core::credentials::TokenCredential;
    use azure_identity::{
        ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId,
        WorkloadIdentityCredential,
    };
    use flow_like_types::async_trait;
    use flow_like_types::base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use p256::ecdsa::VerifyingKey;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    use super::remote::{Backend, http_client, json_body, raw_low_s};
    use crate::audit::crypto::Hash;
    use crate::audit::signer::RawSignature;

    const API_VERSION: &str = "7.4";

    pub(super) struct Azure {
        http: reqwest::Client,
        credential: Arc<dyn TokenCredential>,
        /// `https://{vault}/keys/{name}/{version}`
        key_url: String,
        scope: String,
        key: VerifyingKey,
    }

    #[derive(Deserialize)]
    struct KeyBundle {
        key: JsonWebKey,
    }

    #[derive(Deserialize)]
    struct JsonWebKey {
        kty: String,
        crv: Option<String>,
        x: Option<String>,
        y: Option<String>,
    }

    #[derive(Deserialize)]
    struct Signed {
        value: String,
    }

    fn base64url(value: &str) -> Option<Vec<u8>> {
        URL_SAFE_NO_PAD.decode(value.trim_end_matches('=')).ok()
    }

    impl Azure {
        /// `key_id` is a versioned key identifier, `https://{vault}/keys/{name}/{version}`.
        /// The version is required so one key id always means one public key. `known`
        /// skips reading the public key.
        pub(super) async fn connect(
            key_id: String,
            known: Option<VerifyingKey>,
        ) -> flow_like_types::Result<Self> {
            let url = reqwest::Url::parse(&key_id)
                .map_err(|_| flow_like_types::anyhow!("AUDIT_KMS_KEY_ID {key_id} is not a URL"))?;
            let segments: Vec<&str> = url
                .path_segments()
                .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
                .unwrap_or_default();
            let (Some(host), ["keys", name, version]) = (url.host_str(), segments.as_slice())
            else {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_KMS_KEY_ID {key_id} must be https://<vault>/keys/<name>/<version>"
                ));
            };
            if url.scheme() != "https" {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_KMS_KEY_ID {key_id} must use https"
                ));
            }
            // `myvault.vault.azure.net` -> `https://vault.azure.net/.default`; the same
            // rule covers managed HSMs and sovereign clouds.
            let Some((_, audience)) = host.split_once('.') else {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_KMS_KEY_ID {key_id} has no vault host"
                ));
            };
            let scope = format!("https://{audience}/.default");
            let key_url = format!("https://{host}/keys/{name}/{version}");

            // AKS Workload Identity mounts a federated token; elsewhere the managed
            // identity answers, user-assigned when AZURE_CLIENT_ID names one.
            let credential: Arc<dyn TokenCredential> = if std::env::var_os(
                "AZURE_FEDERATED_TOKEN_FILE",
            )
            .is_some()
            {
                WorkloadIdentityCredential::new(None).map_err(|error| {
                        flow_like_types::anyhow!(
                            "configuring the workload identity for Key Vault key {key_id} failed: {error}"
                        )
                    })?
            } else {
                let client_id = crate::storage_config::non_empty_env("AZURE_CLIENT_ID");
                ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
                        user_assigned_id: client_id.map(UserAssignedId::ClientId),
                        ..Default::default()
                    }))
                    .map_err(|error| {
                        flow_like_types::anyhow!(
                            "configuring the managed identity for Key Vault key {key_id} failed: {error}"
                        )
                    })?
            };
            let http = http_client()?;
            if let Some(key) = known {
                return Ok(Self {
                    http,
                    credential,
                    key_url,
                    scope,
                    key,
                });
            }

            let token = access_token(&credential, &scope, &key_url).await?;
            let request = http
                .get(format!("{key_url}?api-version={API_VERSION}"))
                .bearer_auth(token)
                .send()
                .await;
            let bundle: KeyBundle =
                json_body(request, &format!("reading Key Vault key {key_url}")).await?;
            let key = verifying_key(&bundle.key).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Key Vault key {key_url} must be an EC P-256 key, got kty {:?} crv {:?}",
                    bundle.key.kty,
                    bundle.key.crv
                )
            })?;
            Ok(Self {
                http,
                credential,
                key_url,
                scope,
                key,
            })
        }
    }

    fn verifying_key(jwk: &JsonWebKey) -> Option<VerifyingKey> {
        if !matches!(jwk.kty.as_str(), "EC" | "EC-HSM") || jwk.crv.as_deref() != Some("P-256") {
            return None;
        }
        let x = base64url(jwk.x.as_deref()?)?;
        let y = base64url(jwk.y.as_deref()?)?;
        if x.len() != 32 || y.len() != 32 {
            return None;
        }
        let mut point = Vec::with_capacity(65);
        point.push(0x04);
        point.extend_from_slice(&x);
        point.extend_from_slice(&y);
        VerifyingKey::from_sec1_bytes(&point).ok()
    }

    async fn access_token(
        credential: &Arc<dyn TokenCredential>,
        scope: &str,
        key_url: &str,
    ) -> flow_like_types::Result<String> {
        credential
            .get_token(&[scope], None)
            .await
            .map(|token| token.token.secret().to_owned())
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "acquiring a Key Vault token for {key_url} from the managed identity failed: {error}"
                )
            })
    }

    #[async_trait]
    impl Backend for Azure {
        fn key_id(&self) -> &str {
            &self.key_url
        }

        fn public_key(&self) -> VerifyingKey {
            self.key
        }

        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            let token = access_token(&self.credential, &self.scope, &self.key_url).await?;
            let body = serde_json::json!({
                "alg": "ES256",
                "value": URL_SAFE_NO_PAD.encode(Sha256::digest(digest)),
            });
            let request = self
                .http
                .post(format!("{}/sign?api-version={API_VERSION}", self.key_url))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await;
            let signed: Signed = json_body(
                request,
                &format!("signing with Key Vault key {}", self.key_url),
            )
            .await?;
            let raw = base64url(&signed.value).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Key Vault returned a signature for {} that is not base64url",
                    self.key_url
                )
            })?;
            raw_low_s(&raw, &self.key_url)
        }
    }
}

/// HashiCorp Vault or OpenBao `transit`: the self-hosted key service, for deployments
/// that want a key they cannot export without depending on a cloud account. Runs in
/// Docker Compose, Kubernetes or on bare metal, and costs nothing per signature.
mod vault {
    use std::path::PathBuf;

    use flow_like_types::async_trait;
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::DecodePublicKey;
    use serde::Deserialize;

    use super::remote::{Backend, client, json_body};
    use crate::audit::crypto::Hash;
    use crate::audit::signer::{RawSignature, raw_from_der};
    use crate::storage_config::non_empty_env;

    const DEFAULT_MOUNT: &str = "transit";
    const KEY_TYPE: &str = "ecdsa-p256";

    pub(super) struct Vault {
        http: reqwest::Client,
        /// `<address>/v1/<mount>`
        base: String,
        key: String,
        token: Token,
        public: VerifyingKey,
        version: i64,
    }

    /// A Vault token, either configured directly or written by a Vault Agent sink,
    /// which renews it in place.
    enum Token {
        Value(String),
        File(PathBuf),
    }

    #[derive(Deserialize)]
    struct KeyResponse {
        data: KeyData,
    }

    #[derive(Deserialize)]
    struct KeyData {
        #[serde(rename = "type")]
        key_type: String,
        latest_version: i64,
        keys: serde_json::Map<String, serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct SignResponse {
        data: SignData,
    }

    #[derive(Deserialize)]
    struct SignData {
        signature: String,
    }

    impl Token {
        fn from_env() -> flow_like_types::Result<Self> {
            if let Some(path) = non_empty_env("AUDIT_VAULT_TOKEN_FILE") {
                return Ok(Self::File(PathBuf::from(path)));
            }
            non_empty_env("AUDIT_VAULT_TOKEN")
                .or_else(|| non_empty_env("VAULT_TOKEN"))
                .map(Self::Value)
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "a Vault audit key needs AUDIT_VAULT_TOKEN or AUDIT_VAULT_TOKEN_FILE"
                    )
                })
        }

        /// Read per request: an agent-renewed token changes in place.
        fn value(&self) -> flow_like_types::Result<String> {
            match self {
                Self::Value(token) => Ok(token.clone()),
                Self::File(path) => std::fs::read_to_string(path)
                    .map(|token| token.trim().to_owned())
                    .map_err(|error| {
                        flow_like_types::anyhow!(
                            "reading the Vault token from {} failed: {error}",
                            path.display()
                        )
                    }),
            }
        }
    }

    impl Vault {
        /// `key_id` is the transit key name, optionally with its mount:
        /// `audit` or `transit/audit`. A registered public key selects its matching
        /// version, which may be older than Vault's latest version.
        pub(super) async fn connect(
            key_id: String,
            known: Option<VerifyingKey>,
        ) -> flow_like_types::Result<Self> {
            let address = super::vault_address().ok_or_else(|| {
                flow_like_types::anyhow!(
                    "a Vault audit key needs AUDIT_VAULT_ADDR, for example https://vault:8200"
                )
            })?;
            Self::open(&address, Token::from_env()?, key_id, known).await
        }

        async fn open(
            address: &str,
            token: Token,
            key_id: String,
            known: Option<VerifyingKey>,
        ) -> flow_like_types::Result<Self> {
            let address = address.trim_end_matches('/').to_owned();
            let (mount, key) = match key_id.rsplit_once('/') {
                Some((mount, key)) => (mount.trim_matches('/').to_owned(), key.to_owned()),
                None => (DEFAULT_MOUNT.to_owned(), key_id.clone()),
            };
            if key.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_KMS_KEY_ID {key_id} must name a transit key, for example transit/audit"
                ));
            }
            // A Vault reached over plain HTTP must stay inside a private network; the
            // token is a bearer credential. Most self-hosted Vaults present a private
            // CA, which the built-in roots do not trust, so AUDIT_VAULT_CA_FILE is the
            // difference between TLS and sending that token in the clear.
            let https_only = !address.starts_with("http://");
            let roots = ca_bundle()?;
            if roots.is_some() && !https_only {
                return Err(flow_like_types::anyhow!(
                    "AUDIT_VAULT_CA_FILE is set but AUDIT_VAULT_ADDR is http://; use https:// so the CA is used"
                ));
            }
            if !https_only {
                tracing::warn!(
                    target: "audit",
                    "the Vault audit key is reached over plain HTTP; the token crosses the network in the clear"
                );
            }
            let http = client(https_only, roots.as_deref())?;
            let base = format!("{address}/v1/{mount}");
            let (version, public) = read_public_key(&http, &base, &key, &token, known).await?;
            Ok(Self {
                http,
                base,
                key,
                token,
                public,
                version,
            })
        }

        fn key_url(&self) -> String {
            format!("{}/keys/{}", self.base, self.key)
        }
    }

    /// PEM bundle of `AUDIT_VAULT_CA_FILE`, read once at connect.
    fn ca_bundle() -> flow_like_types::Result<Option<Vec<u8>>> {
        let Some(path) = non_empty_env("AUDIT_VAULT_CA_FILE") else {
            return Ok(None);
        };
        std::fs::read(&path).map(Some).map_err(|error| {
            flow_like_types::anyhow!("reading the Vault CA bundle from {path} failed: {error}")
        })
    }

    async fn read_public_key(
        http: &reqwest::Client,
        base: &str,
        key: &str,
        token: &Token,
        known: Option<VerifyingKey>,
    ) -> flow_like_types::Result<(i64, VerifyingKey)> {
        let url = format!("{base}/keys/{key}");
        let request = http
            .get(&url)
            .header("X-Vault-Token", token.value()?)
            .send()
            .await;
        let response: KeyResponse =
            json_body(request, &format!("reading the Vault transit key {url}")).await?;
        if response.data.key_type != KEY_TYPE {
            return Err(flow_like_types::anyhow!(
                "Vault transit key {url} must be of type {KEY_TYPE}, got {}",
                response.data.key_type
            ));
        }
        let read_version = |version: i64| -> Option<VerifyingKey> {
            let pem = response
                .data
                .keys
                .get(&version.to_string())?
                .get("public_key")?
                .as_str()?;
            VerifyingKey::from_public_key_pem(pem).ok()
        };
        if let Some(known) = known {
            // A pre-registered key remains tied to its original version after rotation.
            let version = response.data.keys.keys()
                .filter_map(|version| version.parse::<i64>().ok())
                .filter(|version| *version > 0)
                .filter(|version| read_version(*version) == Some(known))
                .max()
                .ok_or_else(|| flow_like_types::anyhow!(
                    "registered public key of Vault transit key {url} matches no retained key version"
                ))?;
            return Ok((version, known));
        }
        let version = response.data.latest_version;
        let public = (version > 0)
            .then(|| read_version(version))
            .flatten()
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Vault transit key {url} has no P-256 public key for v{version}"
                )
            })?;
        Ok((version, public))
    }

    #[async_trait]
    impl Backend for Vault {
        fn key_id(&self) -> &str {
            &self.key
        }

        fn public_key(&self) -> VerifyingKey {
            self.public
        }

        async fn sign(&self, digest: &Hash) -> flow_like_types::Result<RawSignature> {
            // Vault hashes the input with SHA-256 and signs it, which is what every
            // other backend does with the same 32 bytes.
            let body = serde_json::json!({
                "input": STANDARD.encode(digest),
                "hash_algorithm": "sha2-256",
                "marshaling_algorithm": "asn1",
                "prehashed": false,
                "key_version": self.version,
            });
            let url = format!("{}/sign/{}", self.base, self.key);
            let request = self
                .http
                .post(&url)
                .header("X-Vault-Token", self.token.value()?)
                .json(&body)
                .send()
                .await;
            let signed: SignResponse = json_body(
                request,
                &format!("signing with Vault transit key {}", self.key_url()),
            )
            .await?;
            let encoded = signed
                .data
                .signature
                .strip_prefix(&format!("vault:v{}:", self.version))
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Vault returned a signature for {} outside pinned version {}",
                        self.key_url(),
                        self.version
                    )
                })?;
            let der = STANDARD.decode(encoded).map_err(|_| {
                flow_like_types::anyhow!(
                    "Vault returned a signature for {} that is not base64",
                    self.key_url()
                )
            })?;
            raw_from_der(&der)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::remote::signer;
        use super::*;
        use p256::ecdsa::{SigningKey, signature::Signer};
        use p256::pkcs8::{EncodePublicKey, LineEnding};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        const TOKEN: &str = "root";

        /// A transit mount holding one key, answering the two calls the backend makes.
        /// `key_type` is what the mount reports, so the type check can be exercised.
        async fn mount(key: SigningKey, key_type: &'static str) -> String {
            rotating_mount(key, key_type).await.0
        }

        async fn rotating_mount(
            key: SigningKey,
            key_type: &'static str,
        ) -> (String, std::sync::Arc<std::sync::atomic::AtomicI64>) {
            let latest = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(2));
            let server_latest = latest.clone();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let request = read_request(&mut stream).await;
                    let body = answer(
                        &request,
                        &key,
                        key_type,
                        server_latest.load(std::sync::atomic::Ordering::SeqCst),
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            });
            (address, latest)
        }

        async fn read_request(stream: &mut TcpStream) -> String {
            let mut raw = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&raw);
                if let Some((head, body)) = text.split_once("\r\n\r\n")
                    && body.len() >= content_length(head)
                {
                    break;
                }
            }
            String::from_utf8_lossy(&raw).into_owned()
        }

        fn content_length(head: &str) -> usize {
            head.lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().ok())?
                })
                .unwrap_or(0)
        }

        fn answer(request: &str, key: &SigningKey, key_type: &str, latest: i64) -> String {
            let (head, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(
                head.to_ascii_lowercase()
                    .contains(&format!("x-vault-token: {TOKEN}")),
                "every transit call carries the token"
            );
            if head.starts_with("GET") {
                let older = pem(&SigningKey::from_slice(&[7; 32]).unwrap());
                return serde_json::json!({
                    "data": {
                        "type": key_type,
                        "latest_version": latest,
                        "keys": {
                            "1": {"public_key": older},
                            "2": {"public_key": pem(key)},
                            "3": {"public_key": pem(&SigningKey::from_slice(&[8; 32]).unwrap())},
                        },
                    }
                })
                .to_string();
            }
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(request["hash_algorithm"], "sha2-256");
            assert_eq!(request["marshaling_algorithm"], "asn1");
            assert_eq!(request["prehashed"], false);
            let input = STANDARD.decode(request["input"].as_str().unwrap()).unwrap();
            // `prehashed: false` means Vault hashes the input with SHA-256 and signs
            // that, which is what signing the digest as a message does.
            let version = request["key_version"].as_i64().unwrap_or(latest);
            let signing_key = match version {
                1 => SigningKey::from_slice(&[7; 32]).unwrap(),
                2 => key.clone(),
                3 => SigningKey::from_slice(&[8; 32]).unwrap(),
                _ => panic!("unknown key version {version}"),
            };
            let signature: p256::ecdsa::Signature = signing_key.sign(&input);
            let signature = STANDARD.encode(signature.to_der().as_bytes());
            serde_json::json!({"data": {"signature": format!("vault:v{version}:{signature}")}})
                .to_string()
        }

        fn pem(key: &SigningKey) -> String {
            key.verifying_key()
                .to_public_key_pem(LineEnding::LF)
                .unwrap()
        }

        #[test]
        fn a_private_ca_bundle_must_hold_a_certificate() {
            assert!(client(true, None).is_ok());
            assert!(
                client(true, Some(b"")).is_err(),
                "an empty bundle would silently fall back to the built-in roots"
            );
            assert!(client(true, Some(b"-----BEGIN CERTIFICATE-----\nnope\n")).is_err());
        }

        #[tokio::test]
        async fn transit_signatures_verify_under_the_latest_key_version() {
            let key = SigningKey::from_slice(&[3; 32]).unwrap();
            let address = mount(key.clone(), KEY_TYPE).await;
            let vault = Vault::open(
                &address,
                Token::Value(TOKEN.to_owned()),
                "transit/audit".to_owned(),
                None,
            )
            .await
            .unwrap();
            assert_eq!(&vault.public_key(), key.verifying_key());

            // The signer checks every signature against the public key before it
            // returns, so this covers the DER decoding and the low-S form too.
            let signer = signer(vault, Some("audit-2026".to_owned()));
            assert_eq!(signer.kid(), "audit-2026");
            assert!(signer.sign(&[9; 32]).await.is_ok());
        }

        #[tokio::test]
        async fn a_key_of_another_type_is_refused_even_when_its_public_half_is_known() {
            let key = SigningKey::from_slice(&[5; 32]).unwrap();
            let address = mount(key.clone(), "ed25519").await;
            let error = Vault::open(
                &address,
                Token::Value(TOKEN.to_owned()),
                "audit".to_owned(),
                None,
            )
            .await
            .err()
            .expect("only P-256 keys produce signatures the verifier accepts");
            assert!(error.to_string().contains(KEY_TYPE), "{error}");

            let known = Vault::open(
                &address,
                Token::Value(TOKEN.to_owned()),
                "audit".to_owned(),
                Some(*key.verifying_key()),
            )
            .await;
            assert!(known.is_err());
        }

        #[tokio::test]
        async fn a_running_signer_keeps_its_version_after_vault_rotates() {
            let key = SigningKey::from_slice(&[3; 32]).unwrap();
            let (address, latest) = rotating_mount(key, KEY_TYPE).await;
            let vault = Vault::open(&address, Token::Value(TOKEN.into()), "audit".into(), None)
                .await
                .unwrap();
            assert_eq!(vault.version, 2);
            latest.store(3, std::sync::atomic::Ordering::SeqCst);
            let signer = signer(vault, None);
            assert!(signer.sign(&[9; 32]).await.is_ok());
        }

        #[tokio::test]
        async fn registered_keys_pin_the_matching_older_version_on_restart() {
            let key = SigningKey::from_slice(&[3; 32]).unwrap();
            let (address, latest) = rotating_mount(key.clone(), KEY_TYPE).await;
            latest.store(3, std::sync::atomic::Ordering::SeqCst);
            let vault = Vault::open(
                &address,
                Token::Value(TOKEN.into()),
                "audit".into(),
                Some(*key.verifying_key()),
            )
            .await
            .unwrap();
            assert_eq!(vault.version, 2);
            assert!(signer(vault, None).sign(&[9; 32]).await.is_ok());
        }

        #[tokio::test]
        async fn a_registered_key_missing_from_vault_is_rejected_at_connect() {
            let address = mount(SigningKey::from_slice(&[3; 32]).unwrap(), KEY_TYPE).await;
            let unknown = SigningKey::from_slice(&[10; 32]).unwrap();
            let result = Vault::open(
                &address,
                Token::Value(TOKEN.into()),
                "audit".into(),
                Some(*unknown.verifying_key()),
            )
            .await;
            assert!(
                result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("matches no retained key version")
            );
        }

        /// Against a real mount, which the mount above only imitates. Start one with
        /// `bao server -dev` and `bao write -f transit/keys/audit type=ecdsa-p256`, then
        /// run with `AUDIT_VAULT_ADDR` and `AUDIT_VAULT_TOKEN` set:
        /// `cargo test -p flow-like-api --lib transit -- --ignored`.
        #[tokio::test]
        #[ignore = "needs a Vault or OpenBao with a transit key named audit"]
        async fn a_real_transit_mount_signs_what_the_verifier_accepts() {
            let address = std::env::var("AUDIT_VAULT_ADDR").expect("AUDIT_VAULT_ADDR");
            let token = std::env::var("AUDIT_VAULT_TOKEN").expect("AUDIT_VAULT_TOKEN");
            let key = std::env::var("AUDIT_KMS_KEY_ID").unwrap_or_else(|_| "transit/audit".into());
            let vault = Vault::open(&address, Token::Value(token.clone()), key.clone(), None)
                .await
                .unwrap();
            let (public, version) = (vault.public_key(), vault.version);
            signer(vault, None).sign(&[42; 32]).await.unwrap();

            // A restarted worker that only knows the registered public key pins the
            // same version and keeps producing signatures that verify under it.
            let restarted = Vault::open(&address, Token::Value(token), key, Some(public))
                .await
                .unwrap();
            assert_eq!(restarted.version, version);
            signer(restarted, None).sign(&[7; 32]).await.unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_follows_the_setting_then_the_key_id_shape() {
        let gcp = "projects/p/locations/l/keyRings/r/cryptoKeys/k/cryptoKeyVersions/1";
        let azure = "https://vault.vault.azure.net/keys/audit/0123";
        let aws = "arn:aws:kms:eu-central-1:123456789012:key/abcd";
        let address = Some("https://vault:8200");
        assert_eq!(Provider::resolve(None, gcp, None).unwrap(), Provider::Gcp);
        assert_eq!(
            Provider::resolve(None, azure, None).unwrap(),
            Provider::Azure
        );
        assert_eq!(Provider::resolve(None, aws, None).unwrap(), Provider::Aws);
        assert_eq!(
            Provider::resolve(None, "alias/audit", None).unwrap(),
            Provider::Aws
        );
        assert_eq!(
            Provider::resolve(Some("AWS"), gcp, address).unwrap(),
            Provider::Aws,
            "the setting wins over a configured Vault"
        );
        assert_eq!(
            Provider::resolve(Some("google"), aws, None).unwrap(),
            Provider::Gcp
        );
        assert_eq!(
            Provider::resolve(None, "transit/audit", address).unwrap(),
            Provider::Vault,
            "a configured Vault address selects Vault"
        );
        assert_eq!(
            Provider::resolve(Some("openbao"), "audit", None).unwrap(),
            Provider::Vault
        );
        assert!(Provider::resolve(Some("hsm"), aws, None).is_err());
    }
}
